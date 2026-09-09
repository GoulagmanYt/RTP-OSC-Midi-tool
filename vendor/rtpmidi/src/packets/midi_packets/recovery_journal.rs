//! Borrowed, bounded recovery journal parsing (RFC 6295 section 5).
use std::io::{Error, ErrorKind, Result};

fn invalid() -> Error {
    Error::from(ErrorKind::InvalidData)
}
fn take<'a>(bytes: &mut &'a [u8], size: usize) -> Result<&'a [u8]> {
    let part = bytes.get(..size).ok_or_else(invalid)?;
    *bytes = &bytes[size..];
    Ok(part)
}
fn length(bytes: &[u8], minimum: usize) -> Result<usize> {
    let head = bytes.get(..2).ok_or_else(invalid)?;
    let length = (usize::from(head[0] & 3) << 8) | usize::from(head[1]);
    if length < minimum {
        return Err(invalid());
    }
    Ok(length)
}

#[derive(Clone, Copy)]
pub(crate) struct Journal<'a> {
    bytes: &'a [u8],
}
impl<'a> Journal<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut remaining = bytes;
        let head = take(&mut remaining, 3)?;
        if head[0] & 0x40 != 0 {
            let size = length(remaining, 2)?;
            take(&mut remaining, size)?;
        }
        let channels = if head[0] & 0x20 != 0 {
            (head[0] & 15) + 1
        } else {
            0
        };
        let mut previous = None;
        for _ in 0..channels {
            let size = length(remaining, 3)?;
            let channel = take(&mut remaining, size)?;
            let number = (channel[0] >> 3) & 15;
            if previous.is_some_and(|previous| number <= previous) {
                return Err(invalid());
            }
            previous = Some(number);
            validate_chapters(channel[2], &channel[3..])?;
        }
        if !remaining.is_empty() {
            return Err(invalid());
        }
        Ok(Self { bytes })
    }

    pub(crate) fn checkpoint(&self) -> u16 {
        u16::from_be_bytes([self.bytes[1], self.bytes[2]])
    }

    /// Notes, pitch wheel and pressure state are recoverable; other forms retain
    /// the caller's conservative reset policy. Parsing never implies recovery.
    pub(crate) fn state_channels(&self) -> Option<impl Iterator<Item = (u8, u8, &'a [u8])>> {
        if self.bytes[0] & 0x50 != 0 {
            return None;
        }
        let mut remaining = &self.bytes[3..];
        while !remaining.is_empty() {
            let size = length(remaining, 3).ok()?;
            if remaining[0] & 4 != 0 || remaining[2] & !0x1B != 0 {
                return None;
            }
            remaining = &remaining[size..];
        }
        let mut remaining = &self.bytes[3..];
        Some(std::iter::from_fn(move || {
            if remaining.is_empty() {
                return None;
            }
            let size = length(remaining, 3).ok()?;
            let section = &remaining[..size];
            remaining = &remaining[size..];
            Some(((section[0] >> 3) & 15, section[2], &section[3..]))
        }))
    }
}

pub(crate) fn note_layout(bytes: &[u8]) -> Result<(usize, usize, usize)> {
    let head = bytes.get(..2).ok_or_else(invalid)?;
    let low = usize::from(head[1] >> 4);
    let high = usize::from(head[1] & 15);
    let count = if head[0] & 127 == 127 && head[1] == 0xF0 {
        128
    } else {
        usize::from(head[0] & 127)
    };
    let off = if low <= high {
        high - low + 1
    } else if low == 15 && high <= 1 {
        0
    } else {
        return Err(invalid());
    };
    Ok((count, low, off))
}

fn validate_chapters(toc: u8, mut bytes: &[u8]) -> Result<()> {
    for bit in [128, 64, 32, 16, 8, 4, 2, 1] {
        if toc & bit == 0 {
            continue;
        }
        let size = match bit {
            128 => 3,
            64 | 4 | 1 => 1 + 2 * (usize::from(*bytes.first().ok_or_else(invalid)? & 127) + 1),
            32 => length(
                bytes,
                if bytes.first().ok_or_else(invalid)? & 0x40 != 0 {
                    3
                } else {
                    2
                },
            )?,
            16 => 2,
            8 => {
                let (notes, low, off) = note_layout(bytes)?;
                let size = 2 + notes * 2 + off;
                let data = bytes.get(..size).ok_or_else(invalid)?;
                let mut seen = 0u128;
                for log in data[2..2 + notes * 2].chunks_exact(2) {
                    let mask = 1u128 << (log[0] & 127);
                    if seen & mask != 0 || log[1] & 127 == 0 {
                        return Err(invalid());
                    }
                    seen |= mask;
                }
                for (index, bits) in data[2 + notes * 2..].iter().enumerate() {
                    for bit in 0..8 {
                        if bits & (0x80 >> bit) != 0
                            && seen & (1u128 << ((low + index) * 8 + bit)) != 0
                        {
                            return Err(invalid());
                        }
                    }
                }
                size
            }
            2 => 1,
            _ => unreachable!(),
        };
        let chapter = take(&mut bytes, size)?;
        if bit == 1 {
            let mut seen = 0u128;
            for log in chapter[1..].chunks_exact(2) {
                let mask = 1u128 << (log[0] & 127);
                if seen & mask != 0 {
                    return Err(invalid());
                }
                seen |= mask;
            }
        }
    }
    if !bytes.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncation_bad_lengths_duplicate_channels_and_note_overlap() {
        // One channel, one NoteOn log, and a NoteOff bitmap for keys 56..63.
        let journal = [0x20, 0, 8, 0, 8, 8, 1, 0x77, 61, 0xE4, 8];
        let parsed = Journal::parse(&journal).unwrap();
        assert_eq!(parsed.checkpoint(), 8);
        assert_eq!(parsed.state_channels().unwrap().count(), 1);
        for end in 0..journal.len() {
            assert!(Journal::parse(&journal[..end]).is_err());
        }
        for (index, value) in [(4, 2), (7, 0x83), (8, 60), (9, 0)] {
            let mut bad = journal;
            bad[index] = value;
            assert!(Journal::parse(&bad).is_err(), "{index}");
        }
        let mut duplicate = journal.to_vec();
        duplicate[0] = 0x21;
        duplicate.extend_from_slice(&journal[3..]);
        assert!(Journal::parse(&duplicate).is_err());
        assert!(Journal::parse(&[0, 0, 0, 0]).is_err());
        assert!(Journal::parse(&[0x40, 0, 0, 0, 1]).is_err());
        assert!(Journal::parse(&[0, 0, 0]).is_ok());
    }
}
