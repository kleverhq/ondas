use super::malformed;
use crate::{Result, backends::Input};
use std::io::SeekFrom;

pub(super) struct Source {
    input: Box<dyn Input>,
    pub len: u64,
}

impl Source {
    pub fn new(mut input: Box<dyn Input>) -> Result<Self> {
        let len = input.seek(SeekFrom::End(0))?;
        Ok(Self { input, len })
    }

    pub fn check(&self, offset: u64, len: u64) -> Result<()> {
        if offset > self.len || len > self.len - offset {
            return Err(malformed(format!("span {offset:#x}+{len} exceeds source")));
        }
        Ok(())
    }

    pub fn bytes(&mut self, offset: u64, len: u64) -> Result<Vec<u8>> {
        self.check(offset, len)?;
        let len = usize::try_from(len).map_err(|_| malformed("span exceeds address space"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(len)
            .map_err(|_| malformed("cannot allocate source span"))?;
        data.resize(len, 0);
        self.input.seek(SeekFrom::Start(offset))?;
        self.input.read_exact(&mut data)?;
        Ok(data)
    }
}

pub(super) struct Cursor<'a> {
    pub data: &'a [u8],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| malformed("span overflow"))?;
        let bytes = self
            .data
            .get(self.at..end)
            .ok_or_else(|| malformed(format!("need {n} bytes at section offset {:#x}", self.at)))?;
        self.at = end;
        Ok(bytes)
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn terminated(&mut self) -> Result<&'a [u8]> {
        let tail = self
            .data
            .get(self.at..)
            .ok_or_else(|| malformed("string offset"))?;
        let n = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| malformed("unterminated string"))?;
        let bytes = self.take(n)?;
        self.take(1)?;
        Ok(bytes)
    }

    pub fn string(&mut self) -> Result<String> {
        let text = std::str::from_utf8(self.terminated()?)
            .map_err(|_| super::unsupported("non-UTF-8 metadata name"))?
            .to_owned();
        Ok(text)
    }

    pub fn count(&mut self, minimum_bytes: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > (self.data.len() - self.at) / minimum_bytes {
            return Err(malformed("record count exceeds section"));
        }
        Ok(n)
    }

    pub fn expect(&mut self, want: u32) -> Result<()> {
        let got = self.u32()?;
        if got != want {
            return Err(super::unsupported(format!(
                "layout word {got:#x}, expected {want:#x}"
            )));
        }
        Ok(())
    }

    pub fn finish(&self) -> Result<()> {
        if self.at != self.data.len() {
            return Err(super::unsupported("trailing type data"));
        }
        Ok(())
    }
}

pub(super) fn at(data: &[u8], offset: usize) -> Result<Cursor<'_>> {
    Ok(Cursor::new(
        data.get(offset..)
            .ok_or_else(|| malformed("section offset"))?,
    ))
}
