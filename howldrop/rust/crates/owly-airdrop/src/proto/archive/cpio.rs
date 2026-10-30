//! CPIO archives: a streaming reader for odc (`070707`) and newc (`070701`),
//! and an odc writer (UP-2, UP-2a).

use std::fmt;

/// Longest entry name accepted, including its terminating NUL.
pub const MAX_NAME: usize = 4096;

pub const TRAILER: &str = "TRAILER!!!";

pub const MODE_TYPE: u32 = 0o170000;
pub const MODE_DIR: u32 = 0o040000;
pub const MODE_FILE: u32 = 0o100000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Odc,
    Newc,
}

impl Format {
    fn header_len(self) -> usize {
        match self {
            Format::Odc => 76,
            Format::Newc => 110,
        }
    }

    /// newc pads the name and the data to 4 bytes; odc doesn't pad.
    fn pad(self, len: u64) -> usize {
        match self {
            Format::Odc => 0,
            Format::Newc => (len.wrapping_neg() % 4) as usize,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub mode: u32,
    pub size: u64,
    pub nlink: u32,
    pub ino: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.mode & MODE_TYPE == MODE_DIR
    }

    pub fn is_file(&self) -> bool {
        self.mode & MODE_TYPE == MODE_FILE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    FormatChanged,
    BadHeader(&'static str),
    NameTooLong,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadMagic => f.write_str("not an odc or newc CPIO archive"),
            Error::FormatChanged => f.write_str("CPIO format changed mid-archive"),
            Error::BadHeader(what) => write!(f, "bad CPIO header: {what}"),
            Error::NameTooLong => f.write_str("CPIO entry name too long"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// A new entry. Its data follows as `Data` events.
    Entry(Entry),
    Data(&'a [u8]),
    /// The current entry's data is complete.
    EntryEnd,
    /// The `TRAILER!!!` entry: the archive is complete. Anything after it is
    /// padding and is ignored.
    Trailer,
}

/// A push-based CPIO reader: feed it bytes, get back events.
#[derive(Debug, Default)]
pub struct Reader {
    format: Option<Format>,
    state: State,
    /// Header and name bytes collected across feeds.
    buf: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Header,
    Name {
        format: Format,
        name_len: usize,
    },
    Data {
        left: u64,
        pad: usize,
    },
    Pad(usize),
    Done,
}

impl Reader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// True between entries, where the archive could end without losing data.
    pub fn at_boundary(&self) -> bool {
        self.state == State::Header && self.buf.is_empty()
    }

    /// Reads from the start of `input`. Returns how many bytes were used and
    /// the event they completed, if any. `(0, None)` means more input is
    /// needed.
    pub fn feed<'a>(&mut self, input: &'a [u8]) -> Result<(usize, Option<Event<'a>>), Error> {
        let mut pos = 0;
        loop {
            let rest = &input[pos..];
            match self.state {
                State::Done => return Ok((input.len(), None)),
                State::Header => {
                    // The magic says how long the rest of the header is.
                    let want = if self.buf.len() < 6 {
                        6
                    } else {
                        magic(&self.buf[..6])?.header_len()
                    };
                    let n = rest.len().min(want - self.buf.len());
                    self.buf.extend_from_slice(&rest[..n]);
                    pos += n;
                    if self.buf.len() < want {
                        return Ok((pos, None));
                    }
                    if want == 6 {
                        continue;
                    }
                    let format = magic(&self.buf[..6])?;
                    if self.format.is_some_and(|f| f != format) {
                        return Err(Error::FormatChanged);
                    }
                    self.format = Some(format);
                    let name_len = name_len(format, &self.buf)?;
                    self.state = State::Name { format, name_len };
                }
                State::Name { format, name_len } => {
                    let header_len = format.header_len();
                    let want = header_len + name_len + format.pad((header_len + name_len) as u64);
                    let n = rest.len().min(want - self.buf.len());
                    self.buf.extend_from_slice(&rest[..n]);
                    pos += n;
                    if self.buf.len() < want {
                        return Ok((pos, None));
                    }
                    let entry = parse(format, &self.buf[..header_len + name_len])?;
                    self.buf.clear();
                    if entry.name == TRAILER {
                        self.state = State::Done;
                        return Ok((pos, Some(Event::Trailer)));
                    }
                    self.state = State::Data {
                        left: entry.size,
                        pad: format.pad(entry.size),
                    };
                    return Ok((pos, Some(Event::Entry(entry))));
                }
                State::Data { left: 0, pad } => {
                    self.state = State::Pad(pad);
                    return Ok((pos, Some(Event::EntryEnd)));
                }
                State::Data { left, pad } => {
                    let n = (rest.len() as u64).min(left) as usize;
                    if n == 0 {
                        return Ok((pos, None));
                    }
                    self.state = State::Data {
                        left: left - n as u64,
                        pad,
                    };
                    return Ok((pos + n, Some(Event::Data(&rest[..n]))));
                }
                State::Pad(0) => self.state = State::Header,
                State::Pad(left) => {
                    let n = rest.len().min(left);
                    if n == 0 {
                        return Ok((pos, None));
                    }
                    pos += n;
                    self.state = State::Pad(left - n);
                }
            }
        }
    }
}

fn magic(bytes: &[u8]) -> Result<Format, Error> {
    match bytes {
        b"070707" => Ok(Format::Odc),
        b"070701" => Ok(Format::Newc),
        _ => Err(Error::BadMagic),
    }
}

/// Field `index` of a header: odc fields are octal, newc fields 8 hex digits.
fn field(format: Format, header: &[u8], index: usize) -> Result<u64, Error> {
    let (start, len, radix) = match format {
        // dev ino mode uid gid nlink rdev: 6 digits each; mtime: 11;
        // namesize: 6; filesize: 11.
        Format::Odc => {
            const OFFSETS: [(usize, usize); 10] = [
                (6, 6),
                (12, 6),
                (18, 6),
                (24, 6),
                (30, 6),
                (36, 6),
                (42, 6),
                (48, 11),
                (59, 6),
                (65, 11),
            ];
            let (start, len) = OFFSETS[index];
            (start, len, 8)
        }
        Format::Newc => (6 + 8 * index, 8, 16),
    };
    let digits = &header[start..start + len];
    let valid = match radix {
        8 => digits.iter().all(|b| (b'0'..=b'7').contains(b)),
        _ => digits.iter().all(u8::is_ascii_hexdigit),
    };
    if !valid {
        return Err(Error::BadHeader("non-numeric field"));
    }
    Ok(u64::from_str_radix(std::str::from_utf8(digits).unwrap(), radix).unwrap())
}

// Field indexes per format.
const ODC: Fields = Fields {
    ino: 1,
    mode: 2,
    uid: 3,
    gid: 4,
    nlink: 5,
    mtime: 7,
    name: 8,
    size: 9,
};
const NEWC: Fields = Fields {
    ino: 0,
    mode: 1,
    uid: 2,
    gid: 3,
    nlink: 4,
    mtime: 5,
    size: 6,
    name: 11,
};

struct Fields {
    ino: usize,
    mode: usize,
    uid: usize,
    gid: usize,
    nlink: usize,
    mtime: usize,
    name: usize,
    size: usize,
}

fn fields(format: Format) -> &'static Fields {
    match format {
        Format::Odc => &ODC,
        Format::Newc => &NEWC,
    }
}

fn name_len(format: Format, header: &[u8]) -> Result<usize, Error> {
    let len = field(format, header, fields(format).name)?;
    if len == 0 {
        return Err(Error::BadHeader("empty name"));
    }
    if len > MAX_NAME as u64 {
        return Err(Error::NameTooLong);
    }
    Ok(len as usize)
}

fn parse(format: Format, bytes: &[u8]) -> Result<Entry, Error> {
    let f = fields(format);
    let (header, name) = bytes.split_at(format.header_len());
    let name = name
        .strip_suffix(&[0])
        .filter(|n| !n.contains(&0))
        .ok_or(Error::BadHeader("name not NUL-terminated"))?;
    let name =
        String::from_utf8(name.to_vec()).map_err(|_| Error::BadHeader("name is not UTF-8"))?;
    let get = |i| field(format, header, i);
    let small = |v: u64| u32::try_from(v).map_err(|_| Error::BadHeader("field out of range"));
    Ok(Entry {
        name,
        mode: small(get(f.mode)?)?,
        size: get(f.size)?,
        nlink: small(get(f.nlink)?)?,
        ino: small(get(f.ino)?)?,
        uid: small(get(f.uid)?)?,
        gid: small(get(f.gid)?)?,
        mtime: get(f.mtime)?,
    })
}

/// An odc header for `entry`, followed by its name. The entry's data goes
/// right after it, unpadded.
pub fn odc_header(entry: &Entry) -> Vec<u8> {
    let name_len = entry.name.len() + 1;
    let mut out = format!(
        "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
        0,
        entry.ino & 0o777777,
        entry.mode & 0o777777,
        entry.uid & 0o777777,
        entry.gid & 0o777777,
        entry.nlink & 0o777777,
        0,
        entry.mtime & 0o77777777777,
        name_len,
        entry.size,
    )
    .into_bytes();
    out.extend_from_slice(entry.name.as_bytes());
    out.push(0);
    out
}

/// The largest file an odc header can describe: 11 octal digits.
pub const ODC_MAX_SIZE: u64 = 0o77777777777;

pub fn odc_trailer() -> Vec<u8> {
    odc_header(&Entry {
        name: TRAILER.into(),
        mode: 0,
        size: 0,
        nlink: 0,
        ino: 0,
        uid: 0,
        gid: 0,
        mtime: 0,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Reads a whole archive into its entries, with each one's data.
    pub(crate) fn read_all(mut input: &[u8]) -> Result<Vec<(Entry, Vec<u8>)>, Error> {
        let mut reader = Reader::new();
        let mut out: Vec<(Entry, Vec<u8>)> = Vec::new();
        loop {
            let (n, event) = reader.feed(input)?;
            input = &input[n..];
            match event {
                Some(Event::Entry(e)) => out.push((e, Vec::new())),
                Some(Event::Data(d)) => out.last_mut().unwrap().1.extend_from_slice(d),
                Some(Event::EntryEnd) => {}
                Some(Event::Trailer) => return Ok(out),
                None if n == 0 => return Err(Error::BadHeader("truncated")),
                None => {}
            }
        }
    }

    pub(crate) fn file(name: &str, data: &[u8], ino: u32) -> (Entry, Vec<u8>) {
        (
            Entry {
                name: name.into(),
                mode: 0o100644,
                size: data.len() as u64,
                nlink: 1,
                ino,
                uid: 501,
                gid: 20,
                mtime: 1700000000,
            },
            data.to_vec(),
        )
    }

    pub(crate) fn odc(entries: &[(Entry, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (entry, data) in entries {
            out.extend(odc_header(entry));
            out.extend_from_slice(data);
        }
        out.extend(odc_trailer());
        out
    }

    #[test]
    fn odc_round_trips() {
        let entries = vec![file("./a.txt", b"hello", 1), file("./empty", b"", 2)];
        assert_eq!(read_all(&odc(&entries)).unwrap(), entries);
    }

    #[test]
    fn reads_byte_by_byte() {
        let entries = vec![file("./a.txt", b"hello", 1), file("./b", b"xy", 2)];
        let archive = odc(&entries);
        let mut reader = Reader::new();
        let mut data = Vec::new();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            let mut byte = &archive[i..i + 1];
            loop {
                let (n, event) = reader.feed(byte).unwrap();
                byte = &byte[n..];
                match event {
                    Some(Event::Entry(e)) => names.push(e.name),
                    Some(Event::Data(d)) => data.extend_from_slice(d),
                    None if byte.is_empty() => break,
                    _ => {}
                }
            }
        }
        assert!(reader.is_done());
        assert_eq!(names, ["./a.txt", "./b"]);
        assert_eq!(data, b"helloxy");
    }

    #[test]
    fn rejects_bad_headers() {
        let mut archive = odc(&[file("./a", b"x", 1)]);
        archive[20] = b'9';
        assert_eq!(
            read_all(&archive),
            Err(Error::BadHeader("non-numeric field"))
        );

        assert_eq!(read_all(b"070702000000"), Err(Error::BadMagic));

        let mut long = odc(&[file("./a", b"x", 1)]);
        long[59..65].copy_from_slice(b"777777");
        assert_eq!(read_all(&long), Err(Error::NameTooLong));

        let mut unterminated = odc(&[file("./a", b"x", 1)]);
        unterminated[76 + 3] = b'x';
        assert_eq!(
            read_all(&unterminated),
            Err(Error::BadHeader("name not NUL-terminated"))
        );
    }
}
