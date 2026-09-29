use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) const MAX_PACKET_MESSAGES: usize = 1024;
const MAX_AHEAD_NS: i128 = 10_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ScheduledMidi {
    pub deadline: Instant,
    pub order: u64,
    pub data: [u8; 3],
    pub len: u8,
    pub generation: u64,
}

fn take<'a>(bytes: &mut &'a [u8], size: usize) -> Result<&'a [u8], &'static str> {
    let part = bytes.get(..size).ok_or("truncated OSC argument")?;
    *bytes = &bytes[size..];
    Ok(part)
}
fn string<'a>(bytes: &mut &'a [u8]) -> Result<&'a str, &'static str> {
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or("unterminated OSC string")?;
    let padded = (end + 4) & !3;
    let part = take(bytes, padded)?;
    if !part[..end].is_ascii() || part[end..].iter().any(|b| *b != 0) {
        return Err("invalid OSC string padding");
    }
    std::str::from_utf8(&part[..end]).map_err(|_| "invalid OSC string")
}

pub(super) fn decode(
    bytes: &[u8],
    wall: SystemTime,
    now: Instant,
    generation: u64,
    out: &mut Vec<ScheduledMidi>,
) -> Result<(), &'static str> {
    out.clear();
    let result = packet(bytes, wall, now, now, None, 0, generation, out);
    if result.is_err() {
        out.clear();
    }
    result
}

fn deadline(
    tag: u64,
    wall: SystemTime,
    now: Instant,
    parent: Instant,
) -> Result<Instant, &'static str> {
    if tag == 1 {
        return Ok(parent);
    }
    let unix = wall
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "invalid system clock")?;
    let ntp_seconds = unix.as_secs().wrapping_add(2_208_988_800) as u32;
    // Choose the nearest NTP era, including the 2036 seconds rollover.
    let seconds = ((tag >> 32) as u32).wrapping_sub(ntp_seconds) as i32;
    let fractional_ns = (u128::from(tag as u32) * 1_000_000_000) >> 32;
    let delta = i128::from(seconds) * 1_000_000_000 + fractional_ns as i128
        - i128::from(unix.subsec_nanos());
    if delta > MAX_AHEAD_NS {
        return Err("OSC deadline exceeds ten seconds");
    }
    let due = if delta <= 0 {
        now
    } else {
        now + Duration::from_nanos(delta as u64)
    };
    if due < parent {
        return Err("nested OSC bundle precedes parent");
    }
    Ok(due)
}

#[allow(clippy::too_many_arguments)]
fn packet(
    bytes: &[u8],
    wall: SystemTime,
    now: Instant,
    parent: Instant,
    parent_tag: Option<u64>,
    depth: usize,
    generation: u64,
    out: &mut Vec<ScheduledMidi>,
) -> Result<(), &'static str> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) || depth > 16 {
        return Err("invalid OSC packet size or nesting");
    }
    let mut remaining = bytes;
    let address = string(&mut remaining)?;
    if address == "#bundle" {
        let tag = u64::from_be_bytes(take(&mut remaining, 8)?.try_into().unwrap());
        if tag != 1 && parent_tag.is_some_and(|parent| (tag.wrapping_sub(parent) as i64) < 0) {
            return Err("nested OSC timetag precedes parent");
        }
        let due = deadline(tag, wall, now, parent)?;
        let inherited_tag = if tag == 1 { parent_tag } else { Some(tag) };
        while !remaining.is_empty() {
            let size = i32::from_be_bytes(take(&mut remaining, 4)?.try_into().unwrap());
            if size <= 0 {
                return Err("invalid OSC bundle element size");
            }
            let element = take(&mut remaining, size as usize)?;
            packet(
                element,
                wall,
                now,
                due,
                inherited_tag,
                depth + 1,
                generation,
                out,
            )?;
        }
        return Ok(());
    }
    if !address.starts_with('/') {
        return Err("invalid OSC address");
    }
    let tags = string(&mut remaining)?;
    if !tags.starts_with(',') {
        return Err("missing OSC type tags");
    }
    let arguments = remaining;
    for tag in tags.bytes().skip(1) {
        match tag {
            b'i' | b'f' | b'm' => {
                take(&mut remaining, 4)?;
            }
            b'h' | b't' | b'd' => {
                take(&mut remaining, 8)?;
            }
            b's' => {
                string(&mut remaining)?;
            }
            b'b' => {
                let size = i32::from_be_bytes(take(&mut remaining, 4)?.try_into().unwrap());
                if size < 0 {
                    return Err("negative OSC blob size");
                }
                let size = size as usize;
                let data = take(&mut remaining, (size + 3) & !3)?;
                if data[size..].iter().any(|b| *b != 0) {
                    return Err("invalid OSC blob padding");
                }
            }
            b'T' | b'F' | b'N' | b'I' => {}
            _ => return Err("unsupported OSC type tag"),
        }
    }
    if !remaining.is_empty() {
        return Err("OSC arguments do not match type tags");
    }
    let mut midi = None;
    if address == "/midi" {
        if tags != ",m" {
            return Err("/midi requires one OSC MIDI argument");
        }
        let status = arguments[1];
        let len = match status {
            0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 3,
            0xC0..=0xDF | 0xF1 | 0xF3 => 2,
            0xF6 | 0xF8 | 0xFA..=0xFC | 0xFE..=0xFF => 1,
            _ => return Err("invalid OSC MIDI status"),
        };
        let mut data = [status, arguments[2], arguments[3]];
        if data[1..usize::from(len)].iter().any(|b| *b >= 128)
            || data[usize::from(len)..].iter().any(|b| *b != 0)
        {
            return Err("invalid OSC MIDI data");
        }
        if status & 0xF0 == 0x90 && data[2] == 0 {
            data[0] = 0x80 | (status & 15);
        }
        midi = Some((data, len));
    } else if let Some(parameter) = address.strip_prefix(crate::osc::PARAMETER_PATH) {
        let target = if parameter == "sustain" {
            Some(None)
        } else {
            parameter
                .parse::<u8>()
                .ok()
                .filter(|index| (1..=88).contains(index))
                .map(Some)
        };
        if let Some(target) = target {
            let pressed = match tags {
                ",i" => match i32::from_be_bytes(arguments.try_into().unwrap()) {
                    0 => false,
                    1 => true,
                    _ => return Err("OSC switch must be zero or one"),
                },
                ",f" => match f32::from_be_bytes(arguments.try_into().unwrap()) {
                    0.0 => false,
                    1.0 => true,
                    _ => return Err("OSC switch must be zero or one"),
                },
                ",T" => true,
                ",F" => false,
                _ => return Err("invalid OSC switch type"),
            };
            midi = Some((
                match target {
                    Some(index) => [
                        if pressed { 0x90 } else { 0x80 },
                        crate::midi::NOTE_MIN + index - 1,
                        if pressed { 127 } else { 0 },
                    ],
                    None => [0xB0, 64, if pressed { 127 } else { 0 }],
                },
                3,
            ));
        }
    }
    if let Some((data, len)) = midi {
        if out.len() >= MAX_PACKET_MESSAGES || out.len() == out.capacity() {
            return Err("too many OSC messages");
        }
        out.push(ScheduledMidi {
            deadline: parent,
            order: out.len() as u64,
            data,
            len,
            generation,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscPacket, OscType};
    fn message() -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: "/avatar/parameters/1".into(),
            args: vec![OscType::Int(1)],
        })
    }
    #[test]
    fn validates_all_atomic_types_and_rejects_bad_padding_and_truncation() {
        let packet = OscPacket::Message(OscMessage {
            addr: "/types".into(),
            args: vec![
                OscType::Int(1),
                OscType::Float(0.5),
                OscType::String("test".into()),
                OscType::Blob(vec![1, 2, 3]),
                OscType::Long(-9),
                OscType::Time((0, 1).into()),
            ],
        });
        let bytes = rosc::encoder::encode(&packet).unwrap();
        let mut out = Vec::with_capacity(MAX_PACKET_MESSAGES);
        let now = Instant::now();
        let wall = SystemTime::now();
        decode(&bytes, wall, now, 0, &mut out).unwrap();
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end], wall, now, 0, &mut out).is_err());
        }
        let mut bytes = rosc::encoder::encode(&message()).unwrap();
        decode(&bytes, wall, now, 0, &mut out).unwrap();
        assert_eq!(out[0].data, [0x90, 21, 127]);
        let end = bytes.iter().position(|b| *b == 0).unwrap();
        bytes[end + 1] = 1;
        assert!(decode(&bytes, wall, now, 0, &mut out).is_err());
        assert!(out.is_empty());
    }
    #[test]
    fn nested_deadlines_and_packet_capacity_are_enforced_atomically() {
        let wall = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let now = Instant::now();
        let child = OscPacket::Bundle(OscBundle {
            timetag: (4_008_988_800, 1 << 30).into(),
            content: vec![message()],
        });
        let parent = OscPacket::Bundle(OscBundle {
            timetag: (4_008_988_800, 1 << 31).into(),
            content: vec![message(), child],
        });
        let mut out = Vec::with_capacity(MAX_PACKET_MESSAGES);
        assert!(decode(
            &rosc::encoder::encode(&parent).unwrap(),
            wall,
            now,
            0,
            &mut out
        )
        .is_err());
        assert!(out.is_empty());
        let oversized = OscPacket::Bundle(OscBundle {
            timetag: (0, 1).into(),
            content: vec![message(); MAX_PACKET_MESSAGES + 1],
        });
        assert!(decode(
            &rosc::encoder::encode(&oversized).unwrap(),
            wall,
            now,
            0,
            &mut out
        )
        .is_err());
        assert!(out.is_empty());
    }

    #[test]
    fn schedules_nested_bundles_in_order_and_rejects_whole_corrupt_bundle() {
        let wall = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let now = Instant::now();
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: (4_008_988_800, 1 << 31).into(),
            content: vec![message(), message()],
        });
        let mut bytes = rosc::encoder::encode(&bundle).unwrap();
        let mut out = Vec::with_capacity(MAX_PACKET_MESSAGES);
        let allocation = out.as_ptr();
        decode(&bytes, wall, now, 5, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].deadline, now + Duration::from_millis(500));
        assert!(out[0].order < out[1].order);
        assert_eq!(out[0].generation, 5);
        bytes.push(0);
        assert!(decode(&bytes, wall, now, 0, &mut out).is_err());
        assert!(out.is_empty());
        assert_eq!(out.as_ptr(), allocation);
        let era_wall = UNIX_EPOCH + Duration::from_secs(2_085_978_496);
        assert_eq!(
            deadline(1u64 << 32, era_wall, now, now).unwrap(),
            now + Duration::from_secs(1)
        );
    }
}
