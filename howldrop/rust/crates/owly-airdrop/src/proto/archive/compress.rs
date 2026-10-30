//! The two `/Upload` compressions: gzip (`application/x-cpio`) and DVZip
//! (`application/x-dvzip`, DVZ-1–3). Decoders are push-based and write into a
//! caller's buffer, so output per call is bounded however well the input
//! compresses.

use std::fmt;
use std::io::Write;

use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};

/// DVZ-2: senders compress 64 KiB pieces.
pub const DVZIP_PIECE: usize = 0x10000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Gzip(&'static str),
    DvZip(&'static str),
    Deflate(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Gzip(what) => write!(f, "bad gzip stream: {what}"),
            Error::DvZip(what) => write!(f, "bad DVZip stream: {what}"),
            Error::Deflate(e) => write!(f, "bad deflate data: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compressor {
    Gzip,
    DvZip,
}

impl Compressor {
    pub fn content_type(self) -> &'static str {
        match self {
            Compressor::Gzip => "application/x-cpio",
            Compressor::DvZip => "application/x-dvzip",
        }
    }

    /// UP-1: the compression a `Content-Type` names.
    pub fn from_content_type(value: &str) -> Option<Self> {
        let essence = value.split(';').next().unwrap().trim();
        if essence.eq_ignore_ascii_case("application/x-cpio") {
            Some(Compressor::Gzip)
        } else if essence.eq_ignore_ascii_case("application/x-dvzip") {
            Some(Compressor::DvZip)
        } else {
            None
        }
    }
}

/// Bytes moved by one decode call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub consumed: usize,
    pub produced: usize,
}

#[derive(Debug)]
pub enum Decoder {
    Gzip(GzipDecoder),
    DvZip(DvZipDecoder),
}

impl Decoder {
    pub fn new(kind: Compressor, max_dvzip_chunk: u32) -> Self {
        match kind {
            Compressor::Gzip => Decoder::Gzip(GzipDecoder::new()),
            Compressor::DvZip => Decoder::DvZip(DvZipDecoder::new(max_dvzip_chunk)),
        }
    }

    /// Decodes from `input` into `out`, which must not be empty. A call that
    /// moves nothing needs more input.
    pub fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Progress, Error> {
        match self {
            Decoder::Gzip(d) => d.decode(input, out),
            Decoder::DvZip(d) => d.decode(input, out),
        }
    }

    /// Whether the stream could end here: between gzip members or DVZip
    /// chunks.
    pub fn at_boundary(&self) -> bool {
        match self {
            Decoder::Gzip(d) => d.at_boundary(),
            Decoder::DvZip(d) => d.at_boundary(),
        }
    }
}

/// Longest gzip member header accepted. FEXTRA, FNAME and FCOMMENT make it
/// variable; real ones are a few dozen bytes.
const MAX_GZIP_HEADER: usize = 4096;

/// RFC 1952. Concatenated members decode as one stream.
#[derive(Debug)]
pub struct GzipDecoder {
    state: GzState,
    buf: Vec<u8>,
    inflate: Decompress,
    crc: Crc,
    /// Whether at least one member has finished.
    members: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GzState {
    Header,
    Deflate,
    Trailer,
}

impl Default for GzipDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl GzipDecoder {
    pub fn new() -> Self {
        GzipDecoder {
            state: GzState::Header,
            buf: Vec::new(),
            inflate: Decompress::new(false),
            crc: Crc::new(),
            members: 0,
        }
    }

    fn at_boundary(&self) -> bool {
        self.members > 0 && self.state == GzState::Header && self.buf.is_empty()
    }

    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Progress, Error> {
        let mut consumed = 0;
        loop {
            let rest = &input[consumed..];
            match self.state {
                GzState::Header => {
                    let n = rest.len().min(MAX_GZIP_HEADER - self.buf.len());
                    let before = self.buf.len();
                    self.buf.extend_from_slice(&rest[..n]);
                    match gzip_header_len(&self.buf)? {
                        Some(len) => {
                            consumed += len - before;
                            self.buf.clear();
                            self.state = GzState::Deflate;
                            self.inflate.reset(false);
                            self.crc.reset();
                        }
                        None if self.buf.len() >= MAX_GZIP_HEADER => {
                            return Err(Error::Gzip("header too long"));
                        }
                        None => {
                            return Ok(Progress {
                                consumed: consumed + n,
                                produced: 0,
                            });
                        }
                    }
                }
                GzState::Deflate => {
                    let (in_before, out_before) =
                        (self.inflate.total_in(), self.inflate.total_out());
                    let status = self
                        .inflate
                        .decompress(rest, out, FlushDecompress::None)
                        .map_err(|e| Error::Deflate(e.to_string()))?;
                    let used = (self.inflate.total_in() - in_before) as usize;
                    let produced = (self.inflate.total_out() - out_before) as usize;
                    self.crc.update(&out[..produced]);
                    consumed += used;
                    if status == Status::StreamEnd {
                        self.state = GzState::Trailer;
                        if produced > 0 {
                            return Ok(Progress { consumed, produced });
                        }
                        continue;
                    }
                    if used == 0 && produced == 0 && !rest.is_empty() {
                        return Err(Error::Gzip("deflate stream stalled"));
                    }
                    return Ok(Progress { consumed, produced });
                }
                GzState::Trailer => {
                    let n = rest.len().min(8 - self.buf.len());
                    self.buf.extend_from_slice(&rest[..n]);
                    consumed += n;
                    if self.buf.len() < 8 {
                        return Ok(Progress {
                            consumed,
                            produced: 0,
                        });
                    }
                    let crc = u32::from_le_bytes(self.buf[..4].try_into().unwrap());
                    let size = u32::from_le_bytes(self.buf[4..].try_into().unwrap());
                    if crc != self.crc.sum() || size != self.crc.amount() {
                        return Err(Error::Gzip("checksum mismatch"));
                    }
                    self.buf.clear();
                    self.members += 1;
                    self.state = GzState::Header;
                    if consumed == input.len() {
                        return Ok(Progress {
                            consumed,
                            produced: 0,
                        });
                    }
                }
            }
        }
    }
}

/// The length of the gzip header at the start of `buf`, once it's complete.
fn gzip_header_len(buf: &[u8]) -> Result<Option<usize>, Error> {
    const FHCRC: u8 = 2;
    const FEXTRA: u8 = 4;
    const FNAME: u8 = 8;
    const FCOMMENT: u8 = 16;
    if buf.len() >= 2 && buf[..2] != [0x1f, 0x8b] {
        return Err(Error::Gzip("bad magic"));
    }
    if buf.len() >= 3 && buf[2] != 8 {
        return Err(Error::Gzip("not deflate"));
    }
    if buf.len() < 10 {
        return Ok(None);
    }
    let flags = buf[3];
    if flags & 0xe0 != 0 {
        return Err(Error::Gzip("reserved flags set"));
    }
    let mut pos = 10;
    if flags & FEXTRA != 0 {
        let Some(len) = buf.get(pos..pos + 2) else {
            return Ok(None);
        };
        pos += 2 + u16::from_le_bytes([len[0], len[1]]) as usize;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let Some(end) = buf.get(pos..).and_then(|b| b.iter().position(|&c| c == 0)) else {
                return Ok(None);
            };
            pos += end + 1;
        }
    }
    if flags & FHCRC != 0 {
        pos += 2;
    }
    Ok((buf.len() >= pos).then_some(pos))
}

/// DVZ-1: chunks of a 4-byte BE length and that many bytes of one complete
/// zlib stream, until the body ends; or, with the length's top bit set, that
/// many bytes stored as they are.
#[derive(Debug)]
pub struct DvZipDecoder {
    max_chunk: u32,
    state: DvState,
    len: Vec<u8>,
    inflate: Decompress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DvState {
    Length,
    Chunk { left: u64 },
    Stored { left: u64 },
}

/// DVZ-1: a chunk length's top bit marks a stored chunk.
const STORED: u32 = 0x8000_0000;

impl DvZipDecoder {
    pub fn new(max_chunk: u32) -> Self {
        DvZipDecoder {
            max_chunk,
            state: DvState::Length,
            len: Vec::with_capacity(4),
            inflate: Decompress::new(true),
        }
    }

    fn at_boundary(&self) -> bool {
        self.state == DvState::Length && self.len.is_empty()
    }

    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Progress, Error> {
        let mut consumed = 0;
        loop {
            let rest = &input[consumed..];
            match self.state {
                DvState::Length => {
                    let n = rest.len().min(4 - self.len.len());
                    self.len.extend_from_slice(&rest[..n]);
                    consumed += n;
                    if self.len.len() < 4 {
                        return Ok(Progress {
                            consumed,
                            produced: 0,
                        });
                    }
                    let raw = u32::from_be_bytes(self.len[..].try_into().unwrap());
                    self.len.clear();
                    let len = raw & !STORED;
                    // DVZ-3: no empty chunks, none past the limit.
                    if len == 0 || len > self.max_chunk {
                        return Err(Error::DvZip("bad chunk length"));
                    }
                    if raw & STORED != 0 {
                        self.state = DvState::Stored { left: len as u64 };
                    } else {
                        self.state = DvState::Chunk { left: len as u64 };
                        self.inflate.reset(true);
                    }
                }
                DvState::Stored { left } => {
                    let n = (rest.len() as u64).min(left).min(out.len() as u64) as usize;
                    out[..n].copy_from_slice(&rest[..n]);
                    consumed += n;
                    let left = left - n as u64;
                    self.state = if left == 0 {
                        DvState::Length
                    } else {
                        DvState::Stored { left }
                    };
                    if n > 0 || rest.is_empty() || out.is_empty() {
                        return Ok(Progress {
                            consumed,
                            produced: n,
                        });
                    }
                }
                DvState::Chunk { left } => {
                    let feed = &rest[..(rest.len() as u64).min(left) as usize];
                    let (in_before, out_before) =
                        (self.inflate.total_in(), self.inflate.total_out());
                    let status = self
                        .inflate
                        .decompress(feed, out, FlushDecompress::None)
                        .map_err(|e| Error::Deflate(e.to_string()))?;
                    let used = (self.inflate.total_in() - in_before) as usize;
                    let produced = (self.inflate.total_out() - out_before) as usize;
                    consumed += used;
                    let left = left - used as u64;
                    self.state = DvState::Chunk { left };
                    if status == Status::StreamEnd {
                        // DVZ-3: the zlib stream must fill the chunk exactly.
                        if left != 0 {
                            return Err(Error::DvZip("data after the chunk's zlib stream"));
                        }
                        self.state = DvState::Length;
                        if produced > 0 {
                            return Ok(Progress { consumed, produced });
                        }
                        continue;
                    }
                    if produced == 0 && used == 0 && (left == 0 || !feed.is_empty()) {
                        return Err(Error::DvZip("chunk is not one complete zlib stream"));
                    }
                    return Ok(Progress { consumed, produced });
                }
            }
        }
    }
}

/// Compresses an archive as it's written. `write` and `finish` return the
/// compressed bytes produced so far.
pub struct Encoder {
    kind: EncoderKind,
}

enum EncoderKind {
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
    DvZip { piece: Vec<u8>, level: Compression },
}

impl Encoder {
    pub fn new(kind: Compressor, level: Compression) -> Self {
        let kind = match kind {
            Compressor::Gzip => EncoderKind::Gzip(flate2::write::GzEncoder::new(Vec::new(), level)),
            Compressor::DvZip => EncoderKind::DvZip {
                piece: Vec::with_capacity(DVZIP_PIECE),
                level,
            },
        };
        Encoder { kind }
    }

    pub fn write(&mut self, mut data: &[u8]) -> Vec<u8> {
        match &mut self.kind {
            EncoderKind::Gzip(gz) => {
                gz.write_all(data).expect("writing to a Vec can't fail");
                std::mem::take(gz.get_mut())
            }
            EncoderKind::DvZip { piece, level } => {
                let mut out = Vec::new();
                while !data.is_empty() {
                    let n = data.len().min(DVZIP_PIECE - piece.len());
                    piece.extend_from_slice(&data[..n]);
                    data = &data[n..];
                    if piece.len() == DVZIP_PIECE {
                        out.extend(dvzip_chunk(piece, *level));
                        piece.clear();
                    }
                }
                out
            }
        }
    }

    pub fn finish(self) -> Vec<u8> {
        match self.kind {
            EncoderKind::Gzip(gz) => gz.finish().expect("writing to a Vec can't fail"),
            EncoderKind::DvZip { piece, level } if !piece.is_empty() => dvzip_chunk(&piece, level),
            EncoderKind::DvZip { .. } => Vec::new(),
        }
    }
}

/// One DVZip chunk: a length, then `piece` as its own zlib stream (DVZ-2).
pub fn dvzip_chunk(piece: &[u8], level: Compression) -> Vec<u8> {
    let mut z = Compress::new(level, true);
    let mut out = vec![0; 4];
    out.reserve(piece.len() + 64);
    loop {
        let before = z.total_in() as usize;
        let status = z
            .compress_vec(&piece[before..], &mut out, FlushCompress::Finish)
            .expect("compressing into a Vec can't fail");
        if status == Status::StreamEnd {
            break;
        }
        out.reserve(out.capacity().max(64));
    }
    let len = (out.len() - 4) as u32;
    out[..4].copy_from_slice(&len.to_be_bytes());
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Decodes a whole stream with a small output buffer.
    pub(crate) fn decode_all(kind: Compressor, mut input: &[u8]) -> Result<Vec<u8>, Error> {
        let mut decoder = Decoder::new(kind, 1 << 20);
        let mut out = Vec::new();
        let mut buf = [0; 1000];
        loop {
            let p = decoder.decode(input, &mut buf)?;
            out.extend_from_slice(&buf[..p.produced]);
            input = &input[p.consumed..];
            if p.consumed == 0 && p.produced == 0 {
                break;
            }
        }
        if !decoder.at_boundary() {
            return Err(Error::DvZip("truncated"));
        }
        Ok(out)
    }

    fn sample() -> Vec<u8> {
        (0..200_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect()
    }

    #[test]
    fn gzip_round_trips() {
        let data = sample();
        let mut enc = Encoder::new(Compressor::Gzip, Compression::fast());
        let mut stream = enc.write(&data[..1000]);
        stream.extend(enc.write(&data[1000..]));
        stream.extend(enc.finish());
        assert_eq!(decode_all(Compressor::Gzip, &stream).unwrap(), data);
    }

    #[test]
    fn gzip_reads_headers_with_optional_fields() {
        let mut gz = flate2::GzBuilder::new()
            .filename("archive.cpio")
            .comment("hi")
            .extra(vec![1, 2, 3])
            .write(Vec::new(), Compression::default());
        gz.write_all(b"hello").unwrap();
        let stream = gz.finish().unwrap();
        assert_eq!(decode_all(Compressor::Gzip, &stream).unwrap(), b"hello");
    }

    #[test]
    fn gzip_rejects_bad_checksums() {
        let mut enc = Encoder::new(Compressor::Gzip, Compression::fast());
        let mut stream = enc.write(b"hello");
        stream.extend(enc.finish());
        let n = stream.len();
        stream[n - 8] ^= 1;
        assert_eq!(
            decode_all(Compressor::Gzip, &stream),
            Err(Error::Gzip("checksum mismatch"))
        );
    }

    // spec: DVZ-2
    #[test]
    fn dvzip_round_trips_in_64k_pieces() {
        let data = sample();
        let mut enc = Encoder::new(Compressor::DvZip, Compression::new(6));
        let mut stream = Vec::new();
        for piece in data.chunks(7777) {
            stream.extend(enc.write(piece));
        }
        stream.extend(enc.finish());
        assert_eq!(decode_all(Compressor::DvZip, &stream).unwrap(), data);
        assert_eq!(chunk_count(&stream), data.len().div_ceil(DVZIP_PIECE));
    }

    pub(crate) fn chunk_count(mut stream: &[u8]) -> usize {
        let mut n = 0;
        while !stream.is_empty() {
            let len = u32::from_be_bytes(stream[..4].try_into().unwrap()) as usize;
            stream = &stream[4 + len..];
            n += 1;
        }
        n
    }

    /// A stored chunk (DVZ-1).
    fn stored_chunk(data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32 | STORED).to_be_bytes().to_vec();
        out.extend_from_slice(data);
        out
    }

    // spec: T-DVZ-2
    #[test]
    fn dvzip_takes_stored_chunks_between_compressed_ones() {
        // As an iPhone sends a photo: the archive's header compressed, the
        // JPEG's bytes stored, the trailer compressed.
        let data = sample();
        let (head, rest) = data.split_at(1000);
        let (middle, tail) = rest.split_at(150_000);
        let mut stream = dvzip_chunk(head, Compression::default());
        for piece in middle.chunks(DVZIP_PIECE) {
            stream.extend(stored_chunk(piece));
        }
        stream.extend(dvzip_chunk(tail, Compression::default()));
        assert_eq!(decode_all(Compressor::DvZip, &stream).unwrap(), data);

        // Fed a byte at a time, into a small buffer.
        let mut decoder = Decoder::new(Compressor::DvZip, 1 << 20);
        let mut out = Vec::new();
        let mut buf = [0; 7];
        for byte in stream.chunks(1) {
            let mut input = byte;
            loop {
                let p = decoder.decode(input, &mut buf).unwrap();
                out.extend_from_slice(&buf[..p.produced]);
                input = &input[p.consumed..];
                if p.consumed == 0 && p.produced == 0 {
                    break;
                }
            }
        }
        assert!(decoder.at_boundary());
        assert_eq!(out, data);

        // A stored chunk cut short.
        let mut cut = stored_chunk(b"hello");
        cut.pop();
        assert_eq!(
            decode_all(Compressor::DvZip, &cut),
            Err(Error::DvZip("truncated"))
        );
    }

    // spec: DVZ-3, DVZ-4
    #[test]
    fn dvzip_rejects_bad_chunks() {
        let good = dvzip_chunk(b"hello", Compression::default());

        // A length running past the end of the body.
        let mut long = good.clone();
        long[3] += 1;
        assert!(decode_all(Compressor::DvZip, &long).is_err());

        // A length shorter than the zlib stream.
        let mut short = good.clone();
        short[3] -= 1;
        short.pop();
        assert!(decode_all(Compressor::DvZip, &short).is_err());

        // Data after the zlib stream, inside the chunk.
        let mut extra = good.clone();
        extra[3] += 1;
        extra.push(0);
        assert_eq!(
            decode_all(Compressor::DvZip, &extra),
            Err(Error::DvZip("data after the chunk's zlib stream"))
        );

        // Past the configured limit, and empty.
        let mut decoder = DvZipDecoder::new(4);
        assert!(decoder.decode(&good, &mut [0; 100]).is_err());
        let mut decoder = DvZipDecoder::new(100);
        assert!(decoder.decode(&[0, 0, 0, 0], &mut [0; 100]).is_err());

        // DVZ-3: a stored chunk is bounded like any other.
        let mut stored = vec![0x80, 0, 0, 5];
        stored.extend_from_slice(b"hello");
        let mut decoder = DvZipDecoder::new(4);
        assert!(decoder.decode(&stored, &mut [0; 100]).is_err());
        assert_eq!(
            decode_all(Compressor::DvZip, &[0x80, 0, 0, 0]),
            Err(Error::DvZip("bad chunk length"))
        );

        // Raw deflate, not zlib.
        let mut raw = vec![0, 0, 0, 7];
        raw.extend_from_slice(&[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00]);
        assert!(decode_all(Compressor::DvZip, &raw).is_err());
    }

    // spec: SEC-3
    #[test]
    fn output_per_call_is_bounded() {
        let zeros = vec![0; 1 << 22];
        let mut enc = Encoder::new(Compressor::Gzip, Compression::best());
        let mut stream = enc.write(&zeros);
        stream.extend(enc.finish());
        let mut decoder = Decoder::new(Compressor::Gzip, 0);
        let mut out = [0; 4096];
        let p = decoder.decode(&stream, &mut out).unwrap();
        assert!(p.produced <= out.len());
    }
}
