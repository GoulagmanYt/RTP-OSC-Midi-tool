//! RFC 6295 A.4: validate bounded parameter logs before interpreting their state.
use std::io::{Error, ErrorKind, Result};

pub(super) fn validate(bytes: &[u8]) -> Result<()> {
    let invalid = || Error::from(ErrorKind::InvalidData);
    let header = bytes.get(..2).ok_or_else(invalid)?;
    let pending = header[0] & 0x40 != 0;
    let active = header[0] & 0x20 != 0;
    let rpn_only = header[0] & 0x10 != 0;
    let nrpn_only = header[0] & 8 != 0;
    let low_only = header[0] & 4 != 0;
    if (pending && active) || (rpn_only && nrpn_only) {
        return Err(invalid());
    }
    let size = (usize::from(header[0] & 3) << 8) | usize::from(header[1]);
    if size != bytes.len() {
        return Err(invalid());
    }
    let mut remaining = bytes.get(2 + usize::from(pending)..).ok_or_else(invalid)?;
    if active && remaining.is_empty() {
        return Err(invalid());
    }
    let compressed = low_only && (rpn_only || nrpn_only);
    let mut seen = [0u64; 512]; // all RPN and NRPN identities, no heap allocation
    while !remaining.is_empty() {
        let header_size = if compressed { 2 } else { 3 };
        let log = remaining.get(..header_size).ok_or_else(invalid)?;
        let number = u16::from(log[0] & 127)
            | if compressed {
                0
            } else {
                u16::from(log[1] & 127) << 7
            };
        let nrpn = if compressed {
            nrpn_only
        } else {
            log[1] & 128 != 0
        };
        if number == 16_383
            || (low_only && number >= 128)
            || (rpn_only && nrpn)
            || (nrpn_only && !nrpn)
        {
            return Err(invalid());
        }
        let key = usize::from(number) + if nrpn { 16_384 } else { 0 };
        let mask = 1u64 << (key % 64);
        if seen[key / 64] & mask != 0 {
            return Err(invalid());
        }
        seen[key / 64] |= mask;
        let toc = log[header_size - 1];
        if (toc & 2 == 0 && toc & 0xF0 != 0) || (toc & 4 == 0 && toc & 8 != 0) {
            return Err(invalid());
        }
        let fields = usize::from(toc & 128 != 0)
            + usize::from(toc & 64 != 0)
            + 2 * usize::from(toc & 32 != 0)
            + 2 * usize::from(toc & 16 != 0)
            + usize::from(toc & 8 != 0);
        remaining = remaining.get(header_size + fields..).ok_or_else(invalid)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParameterLog {
    pub key: u16,
    pub value_tool: bool,
    pub msb: Option<u8>,
    pub lsb: Option<u8>,
    pub buttons: Option<i16>,
    pub count: Option<u8>,
}

// Only constructed from a validated Chapter M slice.
pub(crate) fn logs(bytes: &[u8]) -> impl Iterator<Item = ParameterLog> + '_ {
    let compressed = bytes[0] & 4 != 0 && bytes[0] & 0x18 != 0;
    let nrpn_only = bytes[0] & 8 != 0;
    let mut remaining = &bytes[2 + usize::from(bytes[0] & 64 != 0)..];
    std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        let header = if compressed { 2 } else { 3 };
        let key = u16::from(remaining[0] & 127)
            | if compressed {
                if nrpn_only { 16384 } else { 0 }
            } else {
                (u16::from(remaining[1] & 127) << 7)
                    | if remaining[1] & 128 != 0 { 16384 } else { 0 }
            };
        let toc = remaining[header - 1];
        remaining = &remaining[header..];
        let mut byte = |flag| {
            if toc & flag != 0 {
                let value = remaining[0] & 127;
                remaining = &remaining[1..];
                Some(value)
            } else {
                None
            }
        };
        let msb = byte(128);
        let lsb = byte(64);
        let buttons = if toc & 32 != 0 {
            let magnitude = i16::from(remaining[0] & 63) * 256 + i16::from(remaining[1]);
            let value = if remaining[0] & 128 != 0 {
                -magnitude
            } else {
                magnitude
            };
            remaining = &remaining[2..];
            Some(value)
        } else {
            None
        };
        if toc & 16 != 0 {
            remaining = &remaining[2..];
        }
        let count = if toc & 8 != 0 {
            let value = remaining[0] & 127;
            remaining = &remaining[1..];
            Some(value)
        } else {
            None
        };
        Some(ParameterLog {
            key,
            value_tool: toc & 2 != 0,
            msb,
            lsb,
            buttons,
            count,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_compressed_pending_and_count_parameter_logs_are_bounded() {
        for bytes in [
            &[0x40, 3, 0x81][..],
            &[0x20, 7, 0, 0, 0xC2, 2, 50],
            &[0x34, 6, 0, 0xC2, 2, 50],
            &[0, 6, 1, 0x80, 0x0C, 0x7F],
            &[0, 9, 1, 0, 0x32, 0x80, 1, 0, 1],
        ] {
            assert!(validate(bytes).is_ok(), "{bytes:?}");
            for end in 0..bytes.len() {
                assert!(validate(&bytes[..end]).is_err());
            }
        }
    }

    #[test]
    fn rejects_contradictory_headers_duplicate_parameters_and_missing_tool_flags() {
        for bytes in [
            &[0x60, 3, 0][..],
            &[0x18, 2],
            &[0x20, 2],
            &[0, 5, 127, 127, 0],
            &[0, 8, 1, 0, 0, 1, 0, 0],
            &[0, 6, 1, 0, 128, 2],
            &[0, 6, 1, 0, 8, 2],
        ] {
            assert!(validate(bytes).is_err(), "{bytes:?}");
        }
    }
}
