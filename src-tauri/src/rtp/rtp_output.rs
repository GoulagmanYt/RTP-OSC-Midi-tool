use crate::{
    bridge::pipeline::midi_reset_generation,
    midi::{is_critical_release_message, MidiFrame},
};
use arc_swap::ArcSwapOption;
use rtpmidi::{
    packets::midi_packets::{midi_event::MidiEvent, rtp_midi_message::RtpMidiMessage},
    sessions::rtp_midi_session::RtpMidiSession,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::oneshot;
impl crate::midi_queue::Priority for OutgoingFrame {
    fn critical(&self) -> bool {
        crate::midi::is_critical_release_message(self.frame.data.as_slice())
    }
}

#[cfg(test)]
use tokio::sync::mpsc;

pub(crate) struct OutgoingFrame {
    frame: MidiFrame,
    generation: u64,
    route_generation: u64,
}
#[derive(Clone, Default)]
pub(crate) struct RtpOutputRoute(Arc<ArcSwapOption<crate::midi_queue::AsyncSender<OutgoingFrame>>>);
impl RtpOutputRoute {
    pub(crate) fn set(&self, sender: Option<crate::midi_queue::AsyncSender<OutgoingFrame>>) {
        self.0.store(sender.map(Arc::new));
    }
    pub(crate) fn send(&self, frame: &MidiFrame) {
        if frame.source.starts_with("RTP:") {
            return;
        }
        let guard = self.0.load();
        let Some(sender) = guard.as_ref() else {
            return;
        };
        let Some(copy) = frame.clone_realtime() else {
            super::rtp_server::record_rtp_drop();
            if is_critical_release_message(frame.data.as_slice()) {
                sender.recovery().request_reset();
            }
            return;
        };
        let accepted = sender
            .try_send(OutgoingFrame {
                generation: copy.generation,
                frame: copy,
                route_generation: sender.recovery().generation(),
            })
            .is_ok();
        if !accepted {
            super::rtp_server::record_rtp_drop();
            if is_critical_release_message(frame.data.as_slice()) {
                sender.recovery().request_reset();
            }
        }
    }
}

pub(crate) async fn run(
    session: Arc<RtpMidiSession>,
    mut input: crate::midi_queue::AsyncReceiver<OutgoingFrame>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut generation = midi_reset_generation();
    let mut route_generation = input.recovery().generation();
    let mut pending = None;
    let mut used = false;
    let mut reset_pending = false;
    let mut tick = tokio::time::interval(Duration::from_millis(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if generation != midi_reset_generation()
            || route_generation != input.recovery().generation()
        {
            route_generation = input.recovery().generation();
            generation = midi_reset_generation();
            reset_pending |= used;
        }
        if reset_pending {
            if matches!(
                tokio::time::timeout(Duration::from_millis(100), send_midi_panic(&session)).await,
                Ok(Ok(()))
            ) {
                used = false;
                reset_pending = false;
            } else {
                tokio::select! { biased; _ = &mut stop => break, _ = tick.tick() => {} }
                continue;
            }
        }
        tokio::select! {
            biased;
            _ = &mut stop => break,
            _ = tick.tick() => {},
            frame = async { if let Some(frame) = pending.take() { Some(frame) } else { input.recv().await } } => {
                let Some(frame) = frame else { break; };
                match crate::midi_queue::generation_action(
                    (frame.generation, frame.route_generation),
                    (generation, route_generation),
                    (midi_reset_generation(), input.recovery().generation()),
                ) {
                    crate::midi_queue::GenerationAction::Discard => continue,
                    crate::midi_queue::GenerationAction::Defer => { pending = Some(frame); continue; }
                    crate::midi_queue::GenerationAction::Ready => {}
                }
                used = true;
                if !matches!(tokio::time::timeout(Duration::from_millis(100), send_bytes(&session, frame.frame.data.as_slice())).await, Ok(Ok(()))) {
                    input.recovery().request_reset();
                }
            }
        }
    }
    if used {
        let _ = tokio::time::timeout(Duration::from_millis(100), send_midi_panic(&session)).await;
    }
}

async fn send_midi_panic(session: &RtpMidiSession) -> std::io::Result<()> {
    use midi_types::{Channel, Control, MidiMessage, Value7};
    let commands: [MidiEvent<'static>; 48] = std::array::from_fn(|index| {
        let channel = Channel::from((index / 3) as u8);
        let control = Control::from([64, 123, 120][index % 3]);
        MidiEvent::new(
            None,
            MidiMessage::ControlChange(channel, control, Value7::from(0)).into(),
        )
    });
    session.send_midi_batch(&commands).await
}

pub(crate) async fn send_bytes(session: &RtpMidiSession, bytes: &[u8]) -> std::io::Result<()> {
    let command = RtpMidiMessage::parse_complete(bytes)?;
    if let RtpMidiMessage::SysEx(payload) = command {
        if payload.len() > 65_536 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "SysEx exceeds 64 KiB payload limit",
            ));
        }
        if payload.len() > 1024 {
            let count = payload.len().div_ceil(1024);
            for (index, data) in payload.chunks(1024).enumerate() {
                let fragment = RtpMidiMessage::SysExSegment {
                    head: if index == 0 { 0xF0 } else { 0xF7 },
                    data,
                    tail: if index + 1 == count { 0xF7 } else { 0xF0 },
                };
                session.send_midi(&fragment).await?;
            }
            return Ok(());
        }
    }
    session.send_midi(&command).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use midi_types::MidiMessage;
    use rtpmidi::sessions::{
        events::event_handling::{MidiMessageEvent, SysExPacketEvent},
        invite_responder::InviteResponder,
    };
    async fn start_session(ssrc: u32) -> (Arc<RtpMidiSession>, u16) {
        for _ in 0..32 {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            if let Ok(session) =
                RtpMidiSession::start(port, "Outbound test", ssrc, InviteResponder::Accept).await
            {
                return (session, port);
            }
        }
        panic!("no RTP port pair available");
    }
    #[test]
    fn route_prevents_echo_and_requests_recovery_on_critical_overflow() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let route = RtpOutputRoute::default();
        let (tx, mut rx) = crate::midi_queue::async_channel(1);
        route.set(Some(tx));
        let mut frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0x90, 60, 100],
            source: Arc::from("RTP:remote"),
        };
        route.send(&frame);
        assert!(rx.try_recv().is_err());
        frame.source = Arc::from("Keyboard");
        route.send(&frame);
        frame.data[0] = 0x80;
        let generation = midi_reset_generation();
        route.send(&frame);
        assert_eq!(midi_reset_generation(), generation);
        assert!(rx.try_recv().unwrap().route_generation < rx.recovery().generation());
        let drops_before = crate::rtp::rtp_server::rtp_dropped_count();
        let old_route = route.0.load();
        route.set(None); // Publication must not wait for readers of the old route.
        route.send(&frame);
        drop(old_route);
        assert_eq!(crate::rtp::rtp_server::rtp_dropped_count(), drops_before);
    }
    #[test]
    fn forwards_wire_midi_segments_large_sysex_and_resets_remote_notes() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let (sender, _) = start_session(71).await;
                let (receiver, port) = start_session(72).await;
                let (notes_tx, mut notes) = mpsc::channel(128);
                receiver.add_listener(MidiMessageEvent, move |(message, _)| { notes_tx.try_send(message).unwrap(); }).await;
                let (sysex_tx, mut sysex) = mpsc::channel(2);
                receiver.add_listener(SysExPacketEvent, move |data| { sysex_tx.try_send(data.to_vec()).unwrap(); }).await;
                sender.invite_participant(format!("127.0.0.1:{port}").parse().unwrap()).await;
                while sender.participants().await.is_empty() { tokio::time::sleep(Duration::from_millis(2)).await; }
                let (tx, rx) = crate::midi_queue::async_channel(32);
                let (stop, cancelled) = oneshot::channel();
                let task = tokio::spawn(run(sender.clone(), rx, cancelled));
                tx.send(OutgoingFrame { frame: MidiFrame { generation: MidiFrame::current_generation(), ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(), data: smallvec::smallvec![0x90, 60, 100], source: Arc::from("Keyboard") }, generation: midi_reset_generation(), route_generation: 0 }).await.unwrap();
                assert!(matches!(notes.recv().await.unwrap(), MidiMessage::NoteOn(_, note, _) if u8::from(note) == 60));
                let mut bytes = vec![0xF0]; bytes.extend((0..2700).map(|index| (index % 128) as u8)); bytes.push(0xF7);
                send_bytes(&sender, &bytes).await.unwrap();
                assert_eq!(sysex.recv().await.unwrap(), bytes[1..bytes.len()-1]);
                crate::bridge::pipeline::request_critical_midi_reset();
                for _ in 0..48 { assert!(matches!(notes.recv().await.unwrap(), MidiMessage::ControlChange(..))); }
                let _ = stop.send(()); task.await.unwrap();
                sender.stop_gracefully().await; receiver.stop_gracefully().await;
            }).await.expect("RTP output regression timed out");
        });
    }
}
