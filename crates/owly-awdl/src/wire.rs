//! Bounds-checked reading, and appending to a buffer.

use alloc::vec::Vec;

use crate::{Error, MacAddr};

/// Reads fields at offsets into a byte slice, failing rather than reading
/// past its end.
#[derive(Clone, Copy)]
pub(crate) struct Bytes<'a>(pub &'a [u8]);

impl<'a> Bytes<'a> {
    pub fn slice(self, at: usize, len: usize) -> Result<&'a [u8], Error> {
        let end = at.checked_add(len).ok_or(Error::Truncated)?;
        self.0.get(at..end).ok_or(Error::Truncated)
    }

    pub fn rest(self, at: usize) -> Result<&'a [u8], Error> {
        self.0.get(at..).ok_or(Error::Truncated)
    }

    pub fn u8(self, at: usize) -> Result<u8, Error> {
        self.0.get(at).copied().ok_or(Error::Truncated)
    }

    pub fn le16(self, at: usize) -> Result<u16, Error> {
        let b = self.slice(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn be16(self, at: usize) -> Result<u16, Error> {
        let b = self.slice(at, 2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn le32(self, at: usize) -> Result<u32, Error> {
        let b = self.slice(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn mac(self, at: usize) -> Result<MacAddr, Error> {
        let b = self.slice(at, 6)?;
        Ok(MacAddr([b[0], b[1], b[2], b[3], b[4], b[5]]))
    }
}

/// Appends little-endian fields.
pub(crate) trait Put {
    fn u8(&mut self, v: u8);
    fn le16(&mut self, v: u16);
    fn be16(&mut self, v: u16);
    fn le32(&mut self, v: u32);
    fn mac(&mut self, m: MacAddr);
}

impl Put for Vec<u8> {
    fn u8(&mut self, v: u8) {
        self.push(v);
    }

    fn le16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn be16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }

    fn le32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn mac(&mut self, m: MacAddr) {
        self.extend_from_slice(&m.0);
    }
}
