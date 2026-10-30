//! A strict binary property list (`bplist00`) reader and writer.
//!
//! Only `bplist00` is accepted, never XML or other versions, and the reader
//! caps input size, nesting depth, and the number of values and bytes it
//! produces, so a hostile plist can't exhaust the stack or memory (SEC-3,
//! SEC-6). Object
//! references may be shared, as Apple's and Python's writers do for repeated
//! values, but never cyclic.

use std::collections::BTreeMap;
use std::fmt;

/// A dictionary. Keys are sorted, so encoding is deterministic.
pub type Dict = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Real(f64),
    /// Seconds since 2001-01-01T00:00:00Z.
    Date(f64),
    Data(Vec<u8>),
    String(String),
    Uid(u64),
    Array(Vec<Value>),
    Dict(Dict),
}

impl Value {
    /// The value for a log: strings escaped and cut at 200 bytes, data as
    /// its length. It may be anyone's, so it's escaped (SEC-4).
    pub fn describe(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Real(r) | Value::Date(r) => r.to_string(),
            Value::Data(d) => format!("<{} bytes>", d.len()),
            Value::String(s) => {
                let cut = s
                    .char_indices()
                    .nth(200)
                    .map_or(s.as_str(), |(i, _)| &s[..i]);
                format!("{cut:?}")
            }
            Value::Uid(u) => format!("uid {u}"),
            Value::Array(a) => {
                let items: Vec<_> = a.iter().map(Value::describe).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Dict(d) => {
                let items: Vec<_> = d
                    .iter()
                    .map(|(k, v)| format!("{k:?}: {}", v.describe()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_data(&self) -> Option<&[u8]> {
        match self {
            Value::Data(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Integer(i)
    }
}

impl From<Vec<u8>> for Value {
    fn from(d: Vec<u8>) -> Self {
        Value::Data(d)
    }
}

impl From<Dict> for Value {
    fn from(d: Dict) -> Self {
        Value::Dict(d)
    }
}

impl From<Vec<Value>> for Value {
    fn from(a: Vec<Value>) -> Self {
        Value::Array(a)
    }
}

/// What the reader accepts.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest encoded plist, in bytes.
    pub max_size: usize,
    /// Deepest nesting of arrays and dictionaries. The top-level value is at
    /// depth 1 (SEC-6).
    pub max_depth: usize,
    /// Most values produced, counting each use of a shared object, so a small
    /// plist can't expand into a huge tree.
    pub max_values: usize,
    /// Most bytes of data and string content produced, counting each use of
    /// a shared object, so a plist can't reference one large object many
    /// times over.
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_size: 4 << 20,
            max_depth: 64,
            max_values: 1 << 16,
            max_bytes: 8 << 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooLarge,
    /// Not `bplist00`: XML, another version, or not a plist.
    NotBplist00,
    TooDeep,
    TooManyValues,
    TooManyBytes,
    Cycle,
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge => f.write_str("plist too large"),
            Error::NotBplist00 => f.write_str("not a bplist00 plist"),
            Error::TooDeep => f.write_str("plist nested too deeply"),
            Error::TooManyValues => f.write_str("plist has too many values"),
            Error::TooManyBytes => f.write_str("plist decodes to too many bytes"),
            Error::Cycle => f.write_str("plist contains a reference cycle"),
            Error::Malformed(what) => write!(f, "malformed plist: {what}"),
        }
    }
}

impl std::error::Error for Error {}

const MAGIC: &[u8; 8] = b"bplist00";
const TRAILER: usize = 32;

/// Parses a `bplist00` document.
pub fn from_bytes(bytes: &[u8], limits: &Limits) -> Result<Value, Error> {
    if bytes.len() > limits.max_size {
        return Err(Error::TooLarge);
    }
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(Error::NotBplist00);
    }
    if bytes.len() < MAGIC.len() + 1 + TRAILER {
        return Err(Error::Malformed("too short"));
    }
    let trailer = &bytes[bytes.len() - TRAILER..];
    let offset_size = trailer[6] as usize;
    let ref_size = trailer[7] as usize;
    let num_objects = be_u64(&trailer[8..16]);
    let top_object = be_u64(&trailer[16..24]);
    let table_offset = be_u64(&trailer[24..32]);
    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
        return Err(Error::Malformed("bad trailer sizes"));
    }
    let body_end = (bytes.len() - TRAILER) as u64;
    if num_objects == 0 || top_object >= num_objects {
        return Err(Error::Malformed("bad object count"));
    }
    // The table holds `num_objects` offsets and sits between the objects
    // and the trailer.
    let table_len = num_objects
        .checked_mul(offset_size as u64)
        .ok_or(Error::Malformed("offset table overflows"))?;
    if table_offset <= MAGIC.len() as u64
        || table_offset
            .checked_add(table_len)
            .is_none_or(|end| end > body_end)
    {
        return Err(Error::Malformed("offset table out of range"));
    }
    let mut reader = Reader {
        bytes,
        offset_size,
        ref_size,
        num_objects: num_objects as usize,
        table_offset: table_offset as usize,
        limits,
        values: 0,
        bytes_out: 0,
        path: Vec::new(),
    };
    reader.object(top_object as usize)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset_size: usize,
    ref_size: usize,
    num_objects: usize,
    table_offset: usize,
    limits: &'a Limits,
    values: usize,
    /// Bytes of data and string content produced so far.
    bytes_out: usize,
    /// Objects on the path from the top-level value, for cycle detection.
    path: Vec<usize>,
}

impl<'a> Reader<'a> {
    fn object(&mut self, index: usize) -> Result<Value, Error> {
        self.values += 1;
        if self.values > self.limits.max_values {
            return Err(Error::TooManyValues);
        }
        let at = self.table_offset + index * self.offset_size;
        let offset = be_u64(&self.bytes[at..at + self.offset_size]);
        if offset < MAGIC.len() as u64 || offset >= self.table_offset as u64 {
            return Err(Error::Malformed("object offset out of range"));
        }
        let offset = offset as usize;
        let marker = self.bytes[offset];
        let (kind, info) = (marker >> 4, marker & 0x0f);
        let pos = offset + 1;
        match kind {
            0x0 => match info {
                0x8 => Ok(Value::Bool(false)),
                0x9 => Ok(Value::Bool(true)),
                _ => Err(Error::Malformed("unsupported singleton")),
            },
            0x1 => {
                let n = 1usize << info;
                let raw = self.slice(pos, n)?;
                match n {
                    1 | 2 | 4 => Ok(Value::Integer(be_u64(raw) as i64)),
                    8 => Ok(Value::Integer(be_u64(raw) as i64)),
                    // Writers use 16 bytes for unsigned values above i64::MAX.
                    16 if raw[..8].iter().all(|&b| b == 0) => i64::try_from(be_u64(&raw[8..]))
                        .map(Value::Integer)
                        .map_err(|_| Error::Malformed("integer out of range")),
                    _ => Err(Error::Malformed("integer out of range")),
                }
            }
            0x2 => match info {
                2 => Ok(Value::Real(
                    f32::from_be_bytes(self.slice(pos, 4)?.try_into().unwrap()) as f64,
                )),
                3 => Ok(Value::Real(f64::from_be_bytes(
                    self.slice(pos, 8)?.try_into().unwrap(),
                ))),
                _ => Err(Error::Malformed("bad real size")),
            },
            0x3 if info == 3 => Ok(Value::Date(f64::from_be_bytes(
                self.slice(pos, 8)?.try_into().unwrap(),
            ))),
            0x4 => {
                let (len, pos) = self.length(info, pos)?;
                let raw = self.slice(pos, len)?;
                self.produce(raw.len())?;
                Ok(Value::Data(raw.to_vec()))
            }
            0x5 => {
                let (len, pos) = self.length(info, pos)?;
                let raw = self.slice(pos, len)?;
                self.produce(raw.len())?;
                if !raw.is_ascii() {
                    return Err(Error::Malformed("non-ASCII byte in ASCII string"));
                }
                Ok(Value::String(String::from_utf8(raw.to_vec()).unwrap()))
            }
            0x6 => {
                let (units, pos) = self.length(info, pos)?;
                let len = units
                    .checked_mul(2)
                    .ok_or(Error::Malformed("string too long"))?;
                let raw = self.slice(pos, len)?;
                // A UTF-16 unit becomes at most 3 bytes of UTF-8.
                self.produce(units.saturating_mul(3))?;
                let units = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&c| u16::from_be_bytes(c));
                char::decode_utf16(units)
                    .collect::<Result<String, _>>()
                    .map(Value::String)
                    .map_err(|_| Error::Malformed("invalid UTF-16"))
            }
            0x8 => {
                let raw = self.slice(pos, info as usize + 1)?;
                if raw.len() > 8 {
                    return Err(Error::Malformed("UID too large"));
                }
                Ok(Value::Uid(be_u64(raw)))
            }
            0xa => {
                let (count, pos) = self.length(info, pos)?;
                let refs = self.refs(pos, count)?;
                self.enter(index)?;
                let mut items = Vec::with_capacity(count.min(1024));
                for r in refs {
                    items.push(self.object(r)?);
                }
                self.path.pop();
                Ok(Value::Array(items))
            }
            0xd => {
                let (count, pos) = self.length(info, pos)?;
                let keys = self.refs(pos, count)?;
                let values = self.refs(pos + count * self.ref_size, count)?;
                self.enter(index)?;
                let mut dict = Dict::new();
                for (k, v) in keys.into_iter().zip(values) {
                    let Value::String(key) = self.object(k)? else {
                        return Err(Error::Malformed("dictionary key is not a string"));
                    };
                    let value = self.object(v)?;
                    if dict.insert(key, value).is_some() {
                        return Err(Error::Malformed("duplicate dictionary key"));
                    }
                }
                self.path.pop();
                Ok(Value::Dict(dict))
            }
            _ => Err(Error::Malformed("unsupported object type")),
        }
    }

    /// Counts `len` bytes against the output budget.
    fn produce(&mut self, len: usize) -> Result<(), Error> {
        self.bytes_out = self.bytes_out.saturating_add(len);
        if self.bytes_out > self.limits.max_bytes {
            return Err(Error::TooManyBytes);
        }
        Ok(())
    }

    fn enter(&mut self, index: usize) -> Result<(), Error> {
        if self.path.contains(&index) {
            return Err(Error::Cycle);
        }
        if self.path.len() + 1 > self.limits.max_depth {
            return Err(Error::TooDeep);
        }
        self.path.push(index);
        Ok(())
    }

    /// Objects end where the offset table starts.
    fn slice(&self, pos: usize, len: usize) -> Result<&'a [u8], Error> {
        pos.checked_add(len)
            .filter(|&end| end <= self.table_offset)
            .map(|end| &self.bytes[pos..end])
            .ok_or(Error::Malformed("object runs past the end"))
    }

    /// A length nibble of 0xF means an integer object follows with the length.
    fn length(&self, info: u8, pos: usize) -> Result<(usize, usize), Error> {
        if info != 0x0f {
            return Ok((info as usize, pos));
        }
        let marker = *self.slice(pos, 1)?.first().unwrap();
        if marker >> 4 != 0x1 || marker & 0x0f > 3 {
            return Err(Error::Malformed("bad length"));
        }
        let n = 1usize << (marker & 0x0f);
        let len = be_u64(self.slice(pos + 1, n)?);
        // No length can exceed the input it's read from.
        if len > self.bytes.len() as u64 {
            return Err(Error::Malformed("length runs past the end"));
        }
        Ok((len as usize, pos + 1 + n))
    }

    fn refs(&self, pos: usize, count: usize) -> Result<Vec<usize>, Error> {
        let len = count
            .checked_mul(self.ref_size)
            .ok_or(Error::Malformed("too many references"))?;
        self.slice(pos, len)?
            .chunks_exact(self.ref_size)
            .map(|r| {
                let r = be_u64(r);
                if r < self.num_objects as u64 {
                    Ok(r as usize)
                } else {
                    Err(Error::Malformed("reference out of range"))
                }
            })
            .collect()
    }
}

fn be_u64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |acc, &b| acc << 8 | b as u64)
}

/// Encodes a value as `bplist00`. Values aren't shared, so the output can be
/// larger than Apple's, but it decodes to the same structure.
pub fn to_bytes(value: &Value) -> Vec<u8> {
    let mut objects = Vec::new();
    flatten(value, &mut objects);
    let count = objects.len();
    let ref_size = int_size(count as u64 - 1);

    let mut out = MAGIC.to_vec();
    let mut offsets = Vec::with_capacity(count);
    for object in &objects {
        offsets.push(out.len() as u64);
        encode(object, ref_size, &mut out);
    }
    let table_offset = out.len() as u64;
    let offset_size = int_size(table_offset);
    for offset in offsets {
        out.extend_from_slice(&offset.to_be_bytes()[8 - offset_size..]);
    }
    out.extend_from_slice(&[0; 6]);
    out.push(offset_size as u8);
    out.push(ref_size as u8);
    out.extend_from_slice(&(count as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&table_offset.to_be_bytes());
    out
}

/// One object in the output, with containers holding their children's
/// object indexes.
enum Flat<'a> {
    Leaf(&'a Value),
    Key(&'a str),
    Array(Vec<usize>),
    Dict(Vec<usize>, Vec<usize>),
}

fn flatten<'a>(value: &'a Value, objects: &mut Vec<Flat<'a>>) -> usize {
    let index = objects.len();
    match value {
        Value::Array(items) => {
            objects.push(Flat::Array(Vec::new()));
            let refs = items.iter().map(|v| flatten(v, objects)).collect();
            objects[index] = Flat::Array(refs);
        }
        Value::Dict(dict) => {
            objects.push(Flat::Dict(Vec::new(), Vec::new()));
            let keys = dict
                .keys()
                .map(|k| {
                    objects.push(Flat::Key(k));
                    objects.len() - 1
                })
                .collect();
            let values = dict.values().map(|v| flatten(v, objects)).collect();
            objects[index] = Flat::Dict(keys, values);
        }
        _ => objects.push(Flat::Leaf(value)),
    }
    index
}

fn encode(object: &Flat, ref_size: usize, out: &mut Vec<u8>) {
    let refs = |refs: &[usize], out: &mut Vec<u8>| {
        for &r in refs {
            out.extend_from_slice(&(r as u64).to_be_bytes()[8 - ref_size..]);
        }
    };
    match object {
        Flat::Key(s) => encode_string(s, out),
        Flat::Array(items) => {
            header(0xa, items.len(), out);
            refs(items, out);
        }
        Flat::Dict(keys, values) => {
            header(0xd, keys.len(), out);
            refs(keys, out);
            refs(values, out);
        }
        Flat::Leaf(value) => match value {
            Value::Bool(false) => out.push(0x08),
            Value::Bool(true) => out.push(0x09),
            Value::Integer(i) => encode_int(*i, out),
            Value::Real(r) => {
                out.push(0x23);
                out.extend_from_slice(&r.to_be_bytes());
            }
            Value::Date(d) => {
                out.push(0x33);
                out.extend_from_slice(&d.to_be_bytes());
            }
            Value::Data(d) => {
                header(0x4, d.len(), out);
                out.extend_from_slice(d);
            }
            Value::String(s) => encode_string(s, out),
            Value::Uid(u) => {
                let n = int_size(*u);
                out.push(0x80 | (n as u8 - 1));
                out.extend_from_slice(&u.to_be_bytes()[8 - n..]);
            }
            Value::Array(_) | Value::Dict(_) => unreachable!("containers are flattened"),
        },
    }
}

fn encode_string(s: &str, out: &mut Vec<u8>) {
    if s.is_ascii() {
        header(0x5, s.len(), out);
        out.extend_from_slice(s.as_bytes());
    } else {
        let units: Vec<u16> = s.encode_utf16().collect();
        header(0x6, units.len(), out);
        for u in units {
            out.extend_from_slice(&u.to_be_bytes());
        }
    }
}

fn header(kind: u8, len: usize, out: &mut Vec<u8>) {
    if len < 0x0f {
        out.push(kind << 4 | len as u8);
    } else {
        out.push(kind << 4 | 0x0f);
        encode_int(len as i64, out);
    }
}

/// Non-negative integers use the smallest of 1, 2, 4 or 8 bytes (all but 8
/// are read as unsigned); negative ones always use 8.
fn encode_int(i: i64, out: &mut Vec<u8>) {
    let n = if i < 0 { 8 } else { int_size(i as u64) };
    out.push(0x10 | n.trailing_zeros() as u8);
    out.extend_from_slice(&i.to_be_bytes()[8 - n..]);
}

/// Bytes needed for an unsigned value: 1, 2, 4 or 8.
fn int_size(v: u64) -> usize {
    match v {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xffff_ffff => 4,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: &[(&str, Value)]) -> Value {
        Value::Dict(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn round_trips() {
        let value = dict(&[
            ("bool", true.into()),
            ("negative", (-5i64).into()),
            ("big", 0x1_0000_0000i64.into()),
            ("data", vec![0u8; 300].into()),
            ("ascii", "hello".into()),
            ("unicode", "John’s iPhone 😀".into()),
            ("real", Value::Real(1.5)),
            ("date", Value::Date(700000000.0)),
            ("uid", Value::Uid(7)),
            (
                "array",
                Value::Array((0..20).map(|i| Value::Integer(i * 1000)).collect()),
            ),
            ("empty", dict(&[])),
        ]);
        let bytes = to_bytes(&value);
        assert_eq!(from_bytes(&bytes, &Limits::default()).unwrap(), value);
    }

    #[test]
    fn many_objects_use_wide_references() {
        let value = Value::Array((0..70000).map(Value::Integer).collect());
        let limits = Limits {
            max_values: 100_000,
            ..Limits::default()
        };
        assert_eq!(from_bytes(&to_bytes(&value), &limits).unwrap(), value);
    }

    // spec: SEC-6
    #[test]
    fn rejects_xml_and_other_versions() {
        let xml =
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict/></plist>";
        assert_eq!(from_bytes(xml, &Limits::default()), Err(Error::NotBplist00));
        let mut v1 = to_bytes(&dict(&[]));
        v1[7] = b'1';
        assert_eq!(from_bytes(&v1, &Limits::default()), Err(Error::NotBplist00));
    }

    /// Arrays nested `depth` levels, built by hand so the writer's own
    /// recursion isn't what's tested.
    pub(crate) fn nested(depth: usize) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut offsets = Vec::new();
        for i in 0..depth {
            offsets.push(out.len() as u64);
            if i + 1 < depth {
                out.push(0xa1);
                out.extend_from_slice(&((i + 1) as u16).to_be_bytes());
            } else {
                out.push(0xa0);
            }
        }
        let table = out.len() as u64;
        for o in offsets {
            out.extend_from_slice(&(o as u16).to_be_bytes());
        }
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&[2, 2]);
        out.extend_from_slice(&(depth as u64).to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&table.to_be_bytes());
        out
    }

    // spec: SEC-6
    #[test]
    fn caps_depth_at_64() {
        assert!(from_bytes(&nested(64), &Limits::default()).is_ok());
        assert_eq!(
            from_bytes(&nested(65), &Limits::default()),
            Err(Error::TooDeep)
        );
        assert_eq!(
            from_bytes(&nested(200), &Limits::default()),
            Err(Error::TooDeep)
        );
    }

    #[test]
    fn rejects_cycles() {
        // Object 0 is an array holding itself.
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&[0xa1, 0]);
        out.extend_from_slice(&[8]);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&[1, 1]);
        out.extend_from_slice(&1u64.to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&10u64.to_be_bytes());
        assert_eq!(from_bytes(&out, &Limits::default()), Err(Error::Cycle));
    }

    // spec: SEC-3
    #[test]
    fn shared_references_cannot_expand_without_bound() {
        // Each array holds the next one twice: 2^40 values from 41 objects.
        let n = 41;
        let mut out = MAGIC.to_vec();
        let mut offsets = Vec::new();
        for i in 0..n {
            offsets.push(out.len() as u8);
            if i + 1 < n {
                out.extend_from_slice(&[0xa2, i as u8 + 1, i as u8 + 1]);
            } else {
                out.push(0x08);
            }
        }
        let table = out.len() as u64;
        out.extend_from_slice(&offsets);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&[1, 1]);
        out.extend_from_slice(&(n as u64).to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&table.to_be_bytes());
        assert_eq!(
            from_bytes(&out, &Limits::default()),
            Err(Error::TooManyValues)
        );
    }

    // spec: SEC-3
    #[test]
    fn shared_data_cannot_expand_without_bound() {
        // An array that references one 64 KiB data object 1000 times: 64 MB
        // of output from a 66 KB plist.
        let n = 1000u16;
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&[0xaf, 0x11]);
        out.extend_from_slice(&n.to_be_bytes());
        for _ in 0..n {
            out.extend_from_slice(&1u16.to_be_bytes());
        }
        let data = out.len() as u32;
        out.extend_from_slice(&[0x4f, 0x12]);
        out.extend_from_slice(&(1u32 << 16).to_be_bytes());
        out.extend(std::iter::repeat_n(0, 1 << 16));
        let table = out.len() as u64;
        out.extend_from_slice(&8u32.to_be_bytes());
        out.extend_from_slice(&data.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&[4, 2]);
        out.extend_from_slice(&2u64.to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&table.to_be_bytes());
        assert_eq!(
            from_bytes(&out, &Limits::default()),
            Err(Error::TooManyBytes)
        );
        let limits = Limits {
            max_bytes: usize::MAX,
            ..Limits::default()
        };
        let Ok(Value::Array(items)) = from_bytes(&out, &limits) else {
            panic!("the same plist should decode without the byte cap");
        };
        assert_eq!(items.len(), n as usize);
    }

    // spec: SEC-3
    #[test]
    fn caps_size() {
        let bytes = to_bytes(&Value::Data(vec![0; 2000]));
        let limits = Limits {
            max_size: 1000,
            ..Limits::default()
        };
        assert_eq!(from_bytes(&bytes, &limits), Err(Error::TooLarge));
    }

    #[test]
    fn rejects_truncation_everywhere() {
        let bytes = to_bytes(&dict(&[
            ("a", "b".into()),
            ("c", Value::Array(vec![1.into()])),
        ]));
        for len in 0..bytes.len() {
            assert!(from_bytes(&bytes[..len], &Limits::default()).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_keys() {
        // {"a": true, "a": false}, sharing the key object.
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&[0xd2, 1, 1, 2, 3]); // obj 0 at 8
        out.extend_from_slice(&[0x51, b'a']); // obj 1 at 13
        out.push(0x09); // obj 2 at 15
        out.push(0x08); // obj 3 at 16
        let table = out.len() as u64;
        out.extend_from_slice(&[8, 13, 15, 16]);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&[1, 1]);
        out.extend_from_slice(&4u64.to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        out.extend_from_slice(&table.to_be_bytes());
        assert_eq!(
            from_bytes(&out, &Limits::default()),
            Err(Error::Malformed("duplicate dictionary key"))
        );
    }
}
