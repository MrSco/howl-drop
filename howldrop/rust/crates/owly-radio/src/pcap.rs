//! pcap files of radiotap frames (link type 127), as `tests/captures/`
//! keeps them: written by `owly capture`, read by the replay tests.

use std::io::{self, Read, Write};

/// Radiotap, then 802.11.
pub const LINKTYPE_RADIOTAP: u32 = 127;
const MAGIC: u32 = 0xa1b2_c3d4;
const SNAPLEN: u32 = 65_535;

/// One captured frame, radiotap header first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Microseconds since the Unix epoch, or since whatever epoch the
    /// writer used.
    pub micros: u64,
    pub data: Vec<u8>,
}

pub struct Writer<W: Write> {
    out: W,
}

impl<W: Write> Writer<W> {
    pub fn new(mut out: W) -> io::Result<Writer<W>> {
        let mut h = Vec::with_capacity(24);
        h.extend_from_slice(&MAGIC.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes());
        h.extend_from_slice(&0i32.to_le_bytes()); // GMT offset
        h.extend_from_slice(&0u32.to_le_bytes()); // accuracy
        h.extend_from_slice(&SNAPLEN.to_le_bytes());
        h.extend_from_slice(&LINKTYPE_RADIOTAP.to_le_bytes());
        out.write_all(&h)?;
        Ok(Writer { out })
    }

    pub fn write(&mut self, micros: u64, data: &[u8]) -> io::Result<()> {
        let len = data.len().min(SNAPLEN as usize) as u32;
        let mut h = Vec::with_capacity(16);
        h.extend_from_slice(&((micros / 1_000_000) as u32).to_le_bytes());
        h.extend_from_slice(&((micros % 1_000_000) as u32).to_le_bytes());
        h.extend_from_slice(&len.to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out.write_all(&h)?;
        self.out.write_all(&data[..len as usize])
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Every record of a pcap file of radiotap frames, little- or big-endian,
/// microsecond or nanosecond timestamps.
pub fn read(mut input: impl Read) -> io::Result<Vec<Record>> {
    let mut all = Vec::new();
    input.read_to_end(&mut all)?;
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let header = all.get(..24).ok_or_else(|| bad("not a pcap file"))?;
    let magic = u32::from_le_bytes(header[..4].try_into().unwrap());
    let (le, nanos) = match magic {
        0xa1b2_c3d4 => (true, false),
        0xd4c3_b2a1 => (false, false),
        0xa1b2_3c4d => (true, true),
        0x4d3c_b2a1 => (false, true),
        _ => return Err(bad("not a pcap file")),
    };
    let u32_at = |b: &[u8], at: usize| {
        let v: [u8; 4] = b[at..at + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    if u32_at(header, 20) != LINKTYPE_RADIOTAP {
        return Err(bad("not a capture of radiotap frames (link type 127)"));
    }
    let mut out = Vec::new();
    let mut at = 24;
    while at < all.len() {
        let h = all
            .get(at..at + 16)
            .ok_or_else(|| bad("a truncated record"))?;
        let (sec, frac, len) = (u32_at(h, 0), u32_at(h, 4), u32_at(h, 8) as usize);
        let data = all
            .get(at + 16..at + 16 + len)
            .ok_or_else(|| bad("a truncated record"))?;
        let frac = if nanos {
            frac as u64 / 1000
        } else {
            frac as u64
        };
        out.push(Record {
            micros: sec as u64 * 1_000_000 + frac,
            data: data.to_vec(),
        });
        at += 16 + len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let mut buf = Vec::new();
        let mut w = Writer::new(&mut buf).unwrap();
        w.write(1_700_000_000_123_456, b"one").unwrap();
        w.write(1_700_000_001_000_000, b"").unwrap();
        w.flush().unwrap();
        let r = read(&buf[..]).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0],
            Record {
                micros: 1_700_000_000_123_456,
                data: b"one".to_vec()
            }
        );
        assert!(read(&buf[..buf.len() - 1]).is_err());
        assert!(read(&b"nonsense"[..]).is_err());
    }
}
