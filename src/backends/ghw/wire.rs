//! Checked access to the GHDL wire format. No native reader is linked here.
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use crate::{Error, Format, Result, backends::Input};

pub(super) struct Wire {
    input: BufReader<Box<dyn Input>>,
    pub(super) offset: u64,
    pub(super) len: u64,
    pub(super) big_endian: bool,
    pub(super) version: u8,
}

impl Wire {
    pub(super) fn new(mut input: Box<dyn Input>) -> Result<Self> {
        let len = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        Ok(Self {
            input: BufReader::new(input),
            offset: 0,
            len,
            big_endian: false,
            version: 0,
        })
    }

    #[cfg(test)]
    pub(super) fn with_buffer_capacity(mut self, capacity: usize) -> Self {
        // Call before parsing or before an absolute seek; this drops read-ahead.
        self.input = BufReader::with_capacity(capacity, self.input.into_inner());
        self
    }

    pub(super) fn malformed(&self, message: impl std::fmt::Display) -> Error {
        Error::Malformed {
            format: Format::Ghw,
            backend: "ghw-native".into(),
            message: format!("byte {}: {message}", self.offset),
        }
    }

    pub(super) fn unsupported(&self, message: impl std::fmt::Display) -> Error {
        Error::Backend {
            backend: "ghw-native".into(),
            operation: "open",
            message: format!("byte {}: unsupported GHW {message}", self.offset),
        }
    }

    pub(super) fn seek(&mut self, offset: u64) -> Result<()> {
        self.input.seek(SeekFrom::Start(offset))?;
        self.offset = offset;
        Ok(())
    }

    /// Keep buffered bytes when indexed payloads are close to the current read.
    pub(super) fn seek_value(&mut self, offset: u64) -> Result<()> {
        if let Ok(delta) = i64::try_from(i128::from(offset) - i128::from(self.offset)) {
            self.input.seek_relative(delta)?;
            self.offset = offset;
            Ok(())
        } else {
            self.seek(offset)
        }
    }

    pub(super) fn at_eof(&mut self) -> Result<bool> {
        Ok(self.input.fill_buf()?.is_empty())
    }

    pub(super) fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut result = [0; N];
        let mut filled = 0;
        while filled < N {
            let bytes = self.input.fill_buf()?;
            if bytes.is_empty() {
                return Err(self.malformed("truncated field"));
            }
            let n = bytes.len().min(N - filled);
            result[filled..filled + n].copy_from_slice(&bytes[..n]);
            self.input.consume(n);
            self.offset += n as u64;
            filled += n;
        }
        Ok(result)
    }

    #[inline]
    pub(super) fn byte(&mut self) -> Result<u8> {
        let Some(&byte) = self.input.fill_buf()?.first() else {
            return Err(self.malformed("truncated field"));
        };
        self.input.consume(1);
        self.offset += 1;
        Ok(byte)
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        let bytes = self.fixed()?;
        Ok(if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }

    pub(super) fn i64(&mut self) -> Result<i64> {
        let bytes = self.fixed()?;
        Ok(if self.big_endian {
            i64::from_be_bytes(bytes)
        } else {
            i64::from_le_bytes(bytes)
        })
    }

    pub(super) fn real_bits(&mut self) -> Result<u64> {
        let bytes = self.fixed()?;
        Ok(if self.big_endian {
            u64::from_be_bytes(bytes)
        } else {
            u64::from_le_bytes(bytes)
        })
    }

    pub(super) fn uleb(&mut self) -> Result<u32> {
        let mut value = 0u32;
        for group in 0..5 {
            let byte = self.byte()?;
            if group == 4 && byte & 0xf0 != 0 {
                return Err(self.malformed("ULEB128 exceeds 32 bits"));
            }
            value |= u32::from(byte & 0x7f) << (7 * group);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(self.malformed("unterminated ULEB128"))
    }

    pub(super) fn sleb(&mut self, bits: u32) -> Result<i64> {
        debug_assert!(bits == 32 || bits == 64);
        let mut value = 0u64;
        for group in 0..bits.div_ceil(7) {
            let byte = self.byte()?;
            let shift = group * 7;
            let remaining = bits - shift;
            if remaining < 7 {
                let sign = 1u8 << (remaining - 1);
                let high_mask = 0x7f & !((1u8 << remaining) - 1);
                let expected = if byte & sign == 0 { 0 } else { high_mask };
                if byte & 0x80 != 0 || byte & high_mask != expected {
                    return Err(self.malformed(format_args!("SLEB128 exceeds {bits} bits")));
                }
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                let consumed = shift + 7;
                if byte & 0x40 != 0 && consumed < 64 {
                    value |= u64::MAX << consumed;
                }
                let signed = value as i64;
                if bits == 32 && i32::try_from(signed).is_err() {
                    return Err(self.malformed("SLEB128 exceeds 32 bits"));
                }
                return Ok(signed);
            }
        }
        Err(self.malformed("unterminated SLEB128"))
    }

    pub(super) fn zeros(&mut self) -> Result<()> {
        if self.fixed::<4>()? != [0; 4] {
            return Err(self.malformed("nonzero reserved section field"));
        }
        Ok(())
    }

    pub(super) fn marker(&mut self, expected: &[u8; 4]) -> Result<()> {
        if self.fixed::<4>()? != *expected {
            return Err(self.malformed(format_args!(
                "expected {} section marker",
                String::from_utf8_lossy(expected)
            )));
        }
        Ok(())
    }

    pub(super) fn header(&mut self) -> Result<()> {
        let header = self.fixed::<16>()?;
        if header.starts_with(&[0x1f, 0x8b]) || header.starts_with(b"BZ") {
            return Err(self.unsupported("outer gzip/bzip2 compression"));
        }
        if &header[..9] != b"GHDLwave\n" || header[9] != 16 || header[15] != 0 {
            return Err(self.malformed("invalid file header"));
        }
        if header[10] != 0 || header[11] > 1 {
            return Err(self.unsupported(format_args!("version {}.{}", header[10], header[11])));
        }
        self.version = header[11];
        self.big_endian = match header[12] {
            1 => false,
            2 => true,
            _ => return Err(self.malformed("invalid header byte order")),
        };
        if header[13..15] != [4, 1] {
            return Err(self.unsupported(format_args!(
                "word/offset layout {}/{}",
                header[13], header[14]
            )));
        }
        Ok(())
    }

    pub(super) fn count(&mut self, context: &str) -> Result<usize> {
        let count = self.u32()?;
        if count > i32::MAX as u32 {
            return Err(self.malformed(format_args!("negative {context} count")));
        }
        Ok(count as usize)
    }

    pub(super) fn strings(&mut self) -> Result<Vec<String>> {
        self.marker(b"STR\0")?;
        self.zeros()?;
        let count = self.count("string")?;
        let size = self.count("decoded string byte")?;
        if count as u64 > self.len.saturating_sub(self.offset) {
            return Err(self.malformed("string count exceeds remaining input"));
        }
        let mut strings = vec![String::new()];
        let mut previous = Vec::<u8>::new();
        let mut prefix = 0usize;
        let mut total = 0usize;
        for _ in 0..count {
            if prefix > previous.len() {
                return Err(self.malformed("string prefix exceeds preceding string"));
            }
            previous.truncate(prefix);
            let terminator = loop {
                let byte = self.byte()?;
                if byte & 0x60 == 0 {
                    break byte;
                }
                if total + previous.len() >= size {
                    return Err(self.malformed("string pool exceeds advertised size"));
                }
                previous.push(byte);
            };
            total = total
                .checked_add(previous.len())
                .ok_or_else(|| self.malformed("string pool size overflow"))?;
            if total > size {
                return Err(self.malformed("string pool exceeds advertised size"));
            }
            // GHDL stores character octets. Latin-1 preserves every source byte.
            strings.push(previous.iter().map(|&b| char::from(b)).collect());
            let mut byte = terminator;
            let mut next_prefix = u64::from(byte & 0x1f);
            let mut shift = 5;
            while byte & 0x80 != 0 {
                byte = self.byte()?;
                if byte & 0x60 != 0 || shift >= 35 {
                    return Err(self.malformed("invalid five-bit string prefix"));
                }
                next_prefix |= u64::from(byte & 0x1f) << shift;
                shift += 5;
            }
            prefix = usize::try_from(next_prefix)
                .map_err(|_| self.malformed("string prefix overflow"))?;
        }
        if prefix != 0 {
            return Err(self.malformed("nonzero final string prefix"));
        }
        self.marker(b"EOS\0")?;
        Ok(strings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn wire(bytes: &[u8], capacity: usize) -> Wire {
        Wire::new(Box::new(BufReader::with_capacity(
            capacity,
            Cursor::new(bytes.to_vec()),
        )))
        .unwrap()
        .with_buffer_capacity(capacity)
    }

    #[test]
    fn fixed_fields_cross_tiny_buffers_and_preserve_real_payloads() {
        for capacity in [1, 2, 4096] {
            for big in [false, true] {
                let mut bytes = b"GHDLwave\n\x10\0\x01\x01\x04\x01\0".to_vec();
                bytes[12] = if big { 2 } else { 1 };
                bytes.extend(if big {
                    0x76543210u32.to_be_bytes()
                } else {
                    0x76543210u32.to_le_bytes()
                });
                bytes.extend(if big {
                    (-1234567890i64).to_be_bytes()
                } else {
                    (-1234567890i64).to_le_bytes()
                });
                for bits in [
                    0x8000000000000000u64,
                    0x7ff0000000000000,
                    0x7ff8000000001234,
                ] {
                    bytes.extend(if big {
                        bits.to_be_bytes()
                    } else {
                        bits.to_le_bytes()
                    });
                }
                let mut input = wire(&bytes, capacity);
                input.header().unwrap();
                assert_eq!(input.u32().unwrap(), 0x76543210);
                assert_eq!(input.i64().unwrap(), -1234567890);
                for bits in [
                    0x8000000000000000u64,
                    0x7ff0000000000000,
                    0x7ff8000000001234,
                ] {
                    assert_eq!(input.real_bits().unwrap(), bits);
                }
                assert_eq!(input.offset, bytes.len() as u64);
                input.seek(16).unwrap();
                assert_eq!(input.u32().unwrap(), 0x76543210);
            }
        }
    }

    #[test]
    fn bounded_leb_values_and_nonminimal_valid_encodings() {
        for (bytes, expected) in [
            (&[0][..], 0),
            (&[0x80, 0][..], 0),
            (&[0x81, 0x80, 0][..], 1),
            (&[0xff, 0xff, 0xff, 0xff, 15][..], u32::MAX),
        ] {
            assert_eq!(wire(bytes, 1).uleb().unwrap(), expected);
        }
        for (bytes, bits, expected) in [
            (&[0x7f][..], 32, -1),
            (&[0x80, 0][..], 64, 0),
            (&[0x80, 0x80, 0x80, 0x80, 0x78][..], 32, i32::MIN as i64),
            (&[0xff, 0xff, 0xff, 0xff, 7][..], 32, i32::MAX as i64),
            (
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f][..],
                64,
                i64::MIN,
            ),
            (
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0][..],
                64,
                i64::MAX,
            ),
        ] {
            assert_eq!(wire(bytes, 2).sleb(bits).unwrap(), expected);
        }
        for bytes in [
            &[0x80][..],
            &[0xff, 0xff, 0xff, 0xff, 16],
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0],
        ] {
            assert!(matches!(
                wire(bytes, 1).uleb(),
                Err(Error::Malformed {
                    format: Format::Ghw,
                    ..
                })
            ));
        }
        for (bytes, bits) in [
            (&[0x80][..], 32),
            (&[0xff, 0xff, 0xff, 0xff, 15][..], 32),
            (
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 1][..],
                64,
            ),
        ] {
            assert!(matches!(
                wire(bytes, 1).sleb(bits),
                Err(Error::Malformed { .. })
            ));
        }
    }

    #[test]
    fn string_prefixes_octets_and_empty_pool() {
        let bytes = b"STR\0\0\0\0\0\x03\0\0\0\x09\0\0\0abc\x02d\0\xe9xy\0EOS\0";
        for capacity in [1, 2, 4096] {
            assert_eq!(
                wire(bytes, capacity).strings().unwrap(),
                ["", "abc", "abd", "éxy"]
            );
        }
        let empty = b"STR\0\0\0\0\0\0\0\0\0\0\0\0\0EOS\0";
        assert_eq!(wire(empty, 1).strings().unwrap(), [""]);
        let mut long = b"STR\0\0\0\0\0\x02\0\0\0\x45\0\0\0".to_vec();
        long.extend([b'a'; 34]);
        long.extend([0x82, 1, b'b', 0]);
        long.extend(b"EOS\0");
        let strings = wire(&long, 1).strings().unwrap();
        assert_eq!(strings[2], "a".repeat(34) + "b");
    }

    #[test]
    fn malformed_fields_and_unknown_layouts_have_distinct_errors() {
        let header = b"GHDLwave\n\x10\0\x01\x01\x04\x01\0";
        for length in 0..16 {
            assert!(matches!(
                wire(&header[..length], 1).header(),
                Err(Error::Malformed { .. })
            ));
        }
        let mut future = header.to_vec();
        future[11] = 2;
        assert!(matches!(
            wire(&future, 1).header(),
            Err(Error::Backend {
                operation: "open",
                ..
            })
        ));
        future[11] = 1;
        future[13] = 8;
        assert!(matches!(
            wire(&future, 1).header(),
            Err(Error::Backend { .. })
        ));
        let bad = b"STR\0\0\0\0\0\x02\0\0\0\x09\0\0\0a\x02b\0EOS\0";
        assert!(matches!(
            wire(bad, 1).strings(),
            Err(Error::Malformed { .. })
        ));
    }
}
