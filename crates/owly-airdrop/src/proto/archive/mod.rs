//! Layers 6 and 7: `/Upload` bodies. A compressed (gzip or DVZip) CPIO
//! archive, unpacked as it streams in, and the extraction rules applied to
//! each entry.

pub mod compress;
pub mod cpio;
pub mod extract;

use std::fmt;

use compress::{Compressor, Decoder};
use extract::{Item, RelPath, Rules};

/// Decompressed bytes handed to the CPIO reader per step.
const BLOCK: usize = 64 << 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Compress(compress::Error),
    Cpio(cpio::Error),
    Extract(extract::Error),
    /// The body ended before the archive's trailer.
    Truncated,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Compress(e) => e.fmt(f),
            Error::Cpio(e) => e.fmt(f),
            Error::Extract(e) => e.fmt(f),
            Error::Truncated => f.write_str("archive truncated"),
        }
    }
}

impl std::error::Error for Error {}

impl From<compress::Error> for Error {
    fn from(e: compress::Error) -> Self {
        Error::Compress(e)
    }
}

impl From<cpio::Error> for Error {
    fn from(e: cpio::Error) -> Self {
        Error::Cpio(e)
    }
}

impl From<extract::Error> for Error {
    fn from(e: extract::Error) -> Self {
        Error::Extract(e)
    }
}

/// What the receiver writes, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unpacked {
    Dir(RelPath),
    /// Create this file; its data follows.
    File(RelPath),
    Data(Vec<u8>),
    EndFile,
    /// The archive's trailer: everything has been written.
    Complete,
}

/// Turns compressed body bytes into [`Unpacked`] steps, one bounded step at a
/// time.
#[derive(Debug)]
pub struct Unpacker {
    decoder: Decoder,
    cpio: cpio::Reader,
    rules: Rules,
    block: Vec<u8>,
    block_len: usize,
    block_pos: usize,
    /// The entry being read: a file to write, or a sidecar to skip.
    writing: Option<bool>,
    decoded: u64,
    /// Most decompressed bytes, headers included (SEC-3).
    max_decoded: u64,
}

impl Unpacker {
    pub fn new(
        kind: Compressor,
        rules: Rules,
        limits: &extract::Limits,
        max_dvzip_chunk: u32,
    ) -> Self {
        // Headers and names add at most this much to the file data.
        let overhead = limits.max_entries as u64 * (110 + cpio::MAX_NAME as u64 + 8);
        Unpacker {
            decoder: Decoder::new(kind, max_dvzip_chunk),
            cpio: cpio::Reader::new(),
            rules,
            block: vec![0; BLOCK],
            block_len: 0,
            block_pos: 0,
            writing: None,
            decoded: 0,
            max_decoded: limits.max_total.saturating_add(overhead),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.cpio.is_done()
    }

    /// Takes compressed bytes from the start of `input`. Returns how many it
    /// used and the next step, or `None` if it needs more input.
    pub fn next(&mut self, input: &[u8]) -> Result<(usize, Option<Unpacked>), Error> {
        let mut consumed = 0;
        loop {
            if self.cpio.is_done() {
                return Ok((consumed, Some(Unpacked::Complete)));
            }
            if self.block_pos < self.block_len {
                let (n, event) = self
                    .cpio
                    .feed(&self.block[self.block_pos..self.block_len])?;
                self.block_pos += n;
                let step = match event {
                    None => None,
                    Some(cpio::Event::Entry(entry)) => match self.rules.check(&entry)? {
                        Item::Root => None,
                        Item::Dir(path) => Some(Unpacked::Dir(path)),
                        Item::File(path) => {
                            self.writing = Some(true);
                            Some(Unpacked::File(path))
                        }
                        Item::Skip => {
                            self.writing = Some(false);
                            None
                        }
                    },
                    Some(cpio::Event::Data(data)) => {
                        (self.writing == Some(true)).then(|| Unpacked::Data(data.to_vec()))
                    }
                    Some(cpio::Event::EntryEnd) => {
                        (self.writing.take() == Some(true)).then_some(Unpacked::EndFile)
                    }
                    Some(cpio::Event::Trailer) => Some(Unpacked::Complete),
                };
                if let Some(step) = step {
                    return Ok((consumed, Some(step)));
                }
                continue;
            }
            let p = self.decoder.decode(&input[consumed..], &mut self.block)?;
            consumed += p.consumed;
            self.decoded += p.produced as u64;
            if self.decoded > self.max_decoded {
                return Err(extract::Error::TooLarge.into());
            }
            self.block_pos = 0;
            self.block_len = p.produced;
            if p.consumed == 0 && p.produced == 0 {
                return Ok((consumed, None));
            }
        }
    }

    /// Called when the body has ended.
    pub fn finish(&self) -> Result<(), Error> {
        if self.cpio.is_done() {
            Ok(())
        } else {
            Err(Error::Truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compress::{Encoder, tests::chunk_count};
    use super::cpio::tests::{file, odc};
    use super::extract::{Allowed, Limits};
    use super::*;
    use flate2::Compression;

    fn rules(names: &[&str], max_total: u64) -> Rules {
        Rules::new(
            names
                .iter()
                .map(|n| Allowed {
                    path: RelPath::parse(n).unwrap(),
                    is_directory: false,
                })
                .collect(),
            Limits {
                max_entries: 100,
                max_total,
            },
        )
    }

    fn compress(kind: Compressor, data: &[u8]) -> Vec<u8> {
        let mut enc = Encoder::new(kind, Compression::fast());
        let mut out = enc.write(data);
        out.extend(enc.finish());
        out
    }

    fn unpack(kind: Compressor, body: &[u8], rules: Rules) -> Result<Vec<Unpacked>, Error> {
        let limits = Limits::default();
        let mut u = Unpacker::new(kind, rules, &limits, 1 << 20);
        let mut steps = Vec::new();
        let mut input = body;
        loop {
            let (n, step) = u.next(input)?;
            input = &input[n..];
            match step {
                Some(Unpacked::Complete) => return Ok(steps),
                Some(step) => steps.push(step),
                None => {
                    u.finish()?;
                    unreachable!()
                }
            }
        }
    }

    #[test]
    fn unpacks_both_compressions() {
        let archive = odc(&[file("./a", b"hello", 1), file("./b", &[7; 200_000], 2)]);
        for kind in [Compressor::Gzip, Compressor::DvZip] {
            let steps =
                unpack(kind, &compress(kind, &archive), rules(&["a", "b"], 1 << 20)).unwrap();
            let data: Vec<u8> = steps
                .iter()
                .filter_map(|s| match s {
                    Unpacked::Data(d) => Some(d.clone()),
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(data.len(), 200_005);
            assert_eq!(steps[0], Unpacked::File(RelPath::parse("a").unwrap()));
        }
        assert_eq!(chunk_count(&compress(Compressor::DvZip, &archive)), 4);
    }

    // spec: DVZ-1
    #[test]
    fn unpacks_stored_dvzip_chunks() {
        let photo: Vec<u8> = (0..150_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let archive = odc(&[file("./IMG_0001.jpg", &photo, 1)]);
        let mut body = Vec::new();
        for piece in archive.chunks(compress::DVZIP_PIECE) {
            body.extend((piece.len() as u32 | 0x8000_0000).to_be_bytes());
            body.extend(piece);
        }
        // As the body arrives: in pieces of every size from 1 to 9000 bytes.
        for size in [1, 3, 4, 5, 1000, 1378, 4096, 9000] {
            let limits = Limits::default();
            let mut u = Unpacker::new(
                Compressor::DvZip,
                rules(&["IMG_0001.jpg"], 1 << 20),
                &limits,
                1 << 20,
            );
            let mut got = Vec::new();
            let mut done = false;
            'pieces: for piece in body.chunks(size) {
                let mut input = piece;
                loop {
                    let (n, step) = u.next(input).unwrap();
                    input = &input[n..];
                    match step {
                        Some(Unpacked::Complete) => {
                            done = true;
                            break 'pieces;
                        }
                        Some(Unpacked::Data(d)) => got.extend(d),
                        Some(_) => {}
                        None => {
                            assert!(
                                input.is_empty(),
                                "pieces of {size}: stuck with {} bytes left",
                                input.len()
                            );
                            break;
                        }
                    }
                }
            }
            assert!(done, "pieces of {size}: never complete");
            assert_eq!(got, photo, "pieces of {size}");
        }
        let steps = unpack(Compressor::DvZip, &body, rules(&["IMG_0001.jpg"], 1 << 20)).unwrap();
        let data: Vec<u8> = steps
            .iter()
            .filter_map(|s| match s {
                Unpacked::Data(d) => Some(d.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(data, photo);
    }

    // spec: UP-7
    #[test]
    fn refuses_entries_not_asked_for() {
        let archive = odc(&[file("./a", b"hello", 1), file("./evil", b"x", 2)]);
        let body = compress(Compressor::Gzip, &archive);
        assert!(matches!(
            unpack(Compressor::Gzip, &body, rules(&["a"], 1 << 20)),
            Err(Error::Extract(extract::Error::NotAsked(_)))
        ));
    }

    // spec: SEC-3
    #[test]
    fn bombs_fail_cleanly() {
        let archive = odc(&[file("./a", &vec![0; 4 << 20], 1)]);
        let body = compress(Compressor::Gzip, &archive);
        assert!(body.len() < 64 << 10);
        assert!(matches!(
            unpack(Compressor::Gzip, &body, rules(&["a"], 1 << 20)),
            Err(Error::Extract(extract::Error::TooLarge))
        ));
    }

    #[test]
    fn truncated_archives_fail() {
        let archive = odc(&[file("./a", b"hello", 1)]);
        let body = compress(Compressor::Gzip, &archive[..archive.len() - 10]);
        assert_eq!(
            unpack(Compressor::Gzip, &body, rules(&["a"], 100)),
            Err(Error::Truncated)
        );
    }
}
