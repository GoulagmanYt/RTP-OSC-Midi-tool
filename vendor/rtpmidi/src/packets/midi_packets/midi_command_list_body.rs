use bytes::BytesMut;

use crate::packets::midi_packets::delta_time::delta_time_size;

use super::midi_event::MidiEvent;

pub(super) trait MidiEventList {
    fn write(&self, buffer: &mut BytesMut, z_flag: bool);
    fn size(&self, z_flag: bool) -> usize;
}

// Specific implementation for slices to avoid lifetime issues
impl<'a> MidiEventList for [MidiEvent<'a>] {
    fn write(&self, buffer: &mut BytesMut, z_flag: bool) {
        let mut write_delta_time = z_flag;
        let mut running_status: Option<u8> = None;
        for command in self.iter() {
            command.write(buffer, running_status, write_delta_time);
            update_running_status(&mut running_status, command.command().status());
            write_delta_time = true;
        }
    }

    fn size(&self, z_flag: bool) -> usize {
        let mut length: usize = 0;
        let mut running_status: Option<u8> = None;
        for (i, command) in self.iter().enumerate() {
            if i > 0 || z_flag {
                length += delta_time_size(command.delta_time())
            }
            if command.command().status() >= 0xF0
                || Some(command.command().status()) != running_status
            {
                length += command.command().len();
            } else {
                length += command.command().len() - 1;
            }
            update_running_status(&mut running_status, command.command().status());
        }

        length
    }
}

fn update_running_status(running: &mut Option<u8>, status: u8) {
    match status {
        0x80..=0xEF => *running = Some(status),
        0xF0..=0xF7 => *running = None,
        _ => {}
    }
}
