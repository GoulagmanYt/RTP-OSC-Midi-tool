//! Structural validation for RFC 6295 Appendix B. No allocation or MIDI replay.
use std::io::{Error, ErrorKind, Result};

fn invalid() -> Error {
    Error::from(ErrorKind::InvalidData)
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    let part = bytes.get(..count).ok_or_else(invalid)?;
    *bytes = &bytes[count..];
    Ok(part)
}

// FIRST terminates with a clear high bit; DATA terminates with a set high bit.
// Locate the delimiter without accumulating an unbounded integer.
fn terminated(bytes: &mut &[u8], high: bool) -> Result<usize> {
    let count = bytes
        .iter()
        .position(|b| (*b & 128 != 0) == high)
        .ok_or_else(invalid)?
        + 1;
    take(bytes, count)?;
    Ok(count)
}

pub(super) fn validate(bytes: &[u8]) -> Result<()> {
    let mut remaining = bytes;
    let header = take(&mut remaining, 2)?;
    let size = (usize::from(header[0] & 3) << 8) | usize::from(header[1]);
    if size != bytes.len() {
        return Err(invalid());
    }
    if header[0] & 64 != 0 {
        let toc = take(&mut remaining, 1)?[0];
        if toc & 127 == 0 {
            return Err(invalid());
        }
        for bit in [64, 32, 16, 8, 4, 2, 1] {
            if toc & bit == 0 {
                continue;
            }
            if bit >= 16 {
                take(&mut remaining, 1)?;
            } else if bit >= 4 {
                let log_header = remaining.get(..2).ok_or_else(invalid)?;
                let length = (usize::from(log_header[0] & 3) << 8) | usize::from(log_header[1]);
                if length < 2 {
                    return Err(invalid());
                }
                let mut log = take(&mut remaining, length)?;
                let flags = take(&mut log, 2)?[0];
                if flags & 64 != 0 {
                    take(&mut log, 1)?;
                }
                if flags & 32 != 0 {
                    let count = terminated(&mut log, true)?;
                    if count.min(3) != usize::from((flags >> 2) & 3) {
                        return Err(invalid());
                    }
                }
                // Future LEGAL fields are opaque by specification; LENGTH
                // bounds them. Without L, no unaccounted bytes are allowed.
                if flags & 16 == 0 && !log.is_empty() {
                    return Err(invalid());
                }
            } else {
                let flags = *remaining.first().ok_or_else(invalid)?;
                let length = usize::from(flags & 31);
                if length < 1 + usize::from(flags & 64 != 0) {
                    return Err(invalid());
                }
                let log = take(&mut remaining, length)?;
                if flags & 32 == 0 && log.len() != 1 + usize::from(flags & 64 != 0) {
                    return Err(invalid());
                }
            }
        }
    }
    if header[0] & 32 != 0 {
        take(&mut remaining, 1)?;
    }
    if header[0] & 16 != 0 {
        let flags = take(&mut remaining, 1)?[0];
        if flags & 16 == 0 && flags & 7 != 0 {
            return Err(invalid());
        }
        take(
            &mut remaining,
            2 * usize::from(flags & 16 != 0) + 3 * usize::from(flags & 8 != 0),
        )?;
    }
    if header[0] & 8 != 0 {
        let flags = take(&mut remaining, 1)?[0];
        let complete = flags & 64 != 0;
        let partial = flags & 32 != 0;
        let reverse = flags & 8 != 0;
        let point = usize::from(flags & 7);
        if (!complete && flags & 16 != 0)
            || (!partial && point != if reverse { 0 } else { 7 })
            || (partial && point == if reverse { 0 } else { 7 })
        {
            return Err(invalid());
        }
        if complete {
            take(&mut remaining, 4)?;
        }
        if partial {
            let field = take(&mut remaining, 4)?;
            for index in 0..8 {
                let unused = if reverse {
                    index < point
                } else {
                    index > point
                };
                let nibble = (field[index / 2] >> (4 * (1 - index % 2))) & 15;
                if unused && nibble != 0 {
                    return Err(invalid());
                }
            }
        }
    }
    if header[0] & 4 != 0 {
        if remaining.is_empty() {
            return Err(invalid());
        }
        while !remaining.is_empty() {
            let flags = take(&mut remaining, 1)?[0];
            take(
                &mut remaining,
                usize::from(flags & 64 != 0) + usize::from(flags & 32 != 0),
            )?;
            if flags & 16 != 0 {
                terminated(&mut remaining, false)?;
            }
            if flags & 8 != 0 {
                terminated(&mut remaining, true)?;
            }
        }
    }
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(flags: u8, body: &[u8]) -> Vec<u8> {
        let length = body.len() + 2;
        let mut bytes = vec![flags | (length >> 8) as u8, length as u8];
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn validates_system_fields_and_rejects_truncated_payloads_with_correct_outer_length() {
        for (flags, body) in [
            (64, &[0x70, 1, 2, 3][..]),
            (64, &[8, 0x24, 3, 0x80]),
            (64, &[2, 0x42, 255]),
            (32, &[1]),
            (16, &[0x18, 1, 2, 3, 4, 5]),
            (8, &[0x47, 1, 2, 3, 4]),
            (8, &[0x22, 0x12, 0x30, 0, 0]),
            (4, &[0x7B, 1, 2, 0x81, 0, 1, 0x82]),
        ] {
            assert!(validate(&journal(flags, body)).is_ok(), "{flags} {body:?}");
            for end in 0..body.len() {
                assert!(
                    validate(&journal(flags, &body[..end])).is_err(),
                    "{flags} {end}"
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_flags_delimiters_and_partial_frame_nibbles() {
        for (flags, body) in [
            (64, &[0][..]),
            (64, &[8, 0x24, 3, 0]),
            (64, &[8, 0x28, 3, 0x80]),
            (64, &[2, 0x40]),
            (16, &[1]),
            (8, &[0x17]),
            (8, &[0x20, 0x11, 0, 0, 0]),
            (4, &[0x10, 0x80]),
            (4, &[8, 1]),
            (0, &[0]),
        ] {
            assert!(validate(&journal(flags, body)).is_err(), "{flags} {body:?}");
        }
        // A future LEGAL field must be skipped, not interpreted as a new log.
        assert!(validate(&journal(64, &[8, 16, 4, 128, 255])).is_ok());
        assert!(validate(&journal(64, &[2, 0x23, 128, 255])).is_ok());
        // FIRST may exceed machine integers; structural validation stays bounded
        // by the datagram length and never overflows an accumulator.
        let mut body = vec![0x13];
        body.extend_from_slice(&[255; 128]);
        body.push(0);
        assert!(validate(&journal(4, &body)).is_ok());
    }
}
