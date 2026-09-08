mod common;

use common::find_consecutive_ports;
use core::panic;
use midi_types::{Channel, MidiMessage, Note, Value7};
use rtpmidi::sessions::events::event_handling::{MidiMessageEvent, ParticipantJoinedEvent};
use rtpmidi::sessions::invite_responder::InviteResponder;
use rtpmidi::sessions::rtp_midi_session::RtpMidiSession;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Notify;
static INTEGRATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_two_session_inter_communication() {
    let _guard = INTEGRATION_LOCK.lock().await;
    let (control_port_1, _midi_port_1) = find_consecutive_ports();
    let ssrc1 = 0x11111111;
    let session1 =
        RtpMidiSession::start(control_port_1, "Session1", ssrc1, InviteResponder::Accept)
            .await
            .expect("Failed to start RTP MIDI session");
    let (control_port_2, _midi_port_2) = loop {
        let candidate = find_consecutive_ports();
        if candidate.0 != control_port_1 && candidate.0 != control_port_1 + 1 {
            break candidate;
        }
    };

    let ssrc2 = 0x22222222;
    let session2 =
        RtpMidiSession::start(control_port_2, "Session2", ssrc2, InviteResponder::Accept)
            .await
            .expect("Failed to start RTP MIDI session");

    let sessions_connected = Arc::new(Notify::new());
    let (session1_message_sender, mut session1_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<MidiMessage>();
    let (session2_message_sender, mut session2_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<MidiMessage>();

    session1
        .add_listener(MidiMessageEvent, move |(message, _delta_time)| {
            session1_message_sender.send(message).unwrap();
        })
        .await;

    let sessions_connected_clone = sessions_connected.clone();
    session1
        .add_listener(ParticipantJoinedEvent, move |_participant| {
            sessions_connected_clone.notify_one();
        })
        .await;

    session2
        .add_listener(MidiMessageEvent, move |(message, _delta_time)| {
            session2_message_sender.send(message).unwrap();
        })
        .await;

    // Dropping a public value clone must not cancel a still-owned session.
    drop(session1.as_ref().clone());
    drop(session2.as_ref().clone());
    // A retained value clone must also survive destruction of the original.
    let retained_session = session1.as_ref().clone();
    drop(session1);
    let session1 = Arc::new(retained_session);

    // Invite each other
    let addr1 = SocketAddr::new("127.0.0.1".parse().unwrap(), control_port_1);
    let addr2 = SocketAddr::new("127.0.0.1".parse().unwrap(), control_port_2);
    session1.invite_participant(addr2).await;

    // wait for the sessions to finish connecting
    sessions_connected.notified().await;

    let session1_participants = session1.participants().await;
    let session2_participants = session2.participants().await;
    assert_eq!(session1_participants.len(), 1);
    assert_eq!(session2_participants.len(), 1);
    assert_eq!(session1_participants[0].addr(), addr2);
    assert_eq!(session2_participants[0].addr(), addr1);

    // Send from session1 to session2
    let note_on = MidiMessage::NoteOn(Channel::C1, Note::from(60), Value7::from(100));
    session1.send_midi(&note_on.into()).await.unwrap();

    let result = session2_message_receiver.recv().await;
    match result.as_ref() {
        Some(MidiMessage::NoteOn(channel, note, velocity)) => {
            if let MidiMessage::NoteOn(expected_channel, expected_note, expected_velocity) = note_on
            {
                assert_eq!(channel, &expected_channel);
                assert_eq!(note, &expected_note);
                assert_eq!(velocity, &expected_velocity);
            } else {
                panic!("Expected a NoteOn message");
            }
        }
        _ => panic!("Expected a NoteOn message"),
    }

    // Send from session2 to session1
    let note_off = MidiMessage::NoteOff(Channel::C1, Note::from(60), Value7::from(0));
    session2.send_midi(&note_off.into()).await.unwrap();

    let result = session1_message_receiver.recv().await;
    match result.as_ref() {
        Some(MidiMessage::NoteOff(channel, note, velocity)) => {
            if let MidiMessage::NoteOff(expected_channel, expected_note, expected_velocity) =
                note_off
            {
                assert_eq!(channel, &expected_channel);
                assert_eq!(note, &expected_note);
                assert_eq!(velocity, &expected_velocity);
            } else {
                panic!("Expected a NoteOff message");
            }
        }
        _ => panic!("Expected a NoteOff message"),
    }
}

#[tokio::test]
async fn malformed_udp_loss_rollover_and_control_disconnect() {
    let _guard = INTEGRATION_LOCK.lock().await;
    use rtpmidi::sessions::events::event_handling::{
        PacketLossEvent, ParticipantLeftEvent, TimestampedMidiMessageEvent,
    };
    use tokio::{
        net::UdpSocket,
        time::{Duration, timeout},
    };
    timeout(Duration::from_secs(5), async {
        let (port, _) = find_consecutive_ports();
        let server = RtpMidiSession::start(port, "Regression", 7, InviteResponder::Accept)
            .await
            .unwrap();
        let (peer_port, _) = find_consecutive_ports();
        let control = UdpSocket::bind(("127.0.0.1", peer_port)).await.unwrap();
        let data = UdpSocket::bind(("127.0.0.1", peer_port + 1)).await.unwrap();
        let (tx, mut events) = tokio::sync::mpsc::channel(32);
        server
            .add_listener(TimestampedMidiMessageEvent, move |event| {
                tx.try_send(event).unwrap();
            })
            .await;
        let (sysex_tx, mut sysex_rx) = tokio::sync::mpsc::channel(8);
        server
            .add_listener(
                rtpmidi::sessions::events::event_handling::SysExPacketEvent,
                move |data| {
                    sysex_tx.try_send(data.to_vec()).unwrap();
                },
            )
            .await;
        let (left_tx, mut left_rx) = tokio::sync::mpsc::channel(1);
        server
            .add_listener(ParticipantLeftEvent, move |_| {
                left_tx.try_send(()).unwrap();
            })
            .await;
        let (fault_tx, mut fault_rx) = tokio::sync::mpsc::channel(4);
        server
            .add_listener(
                rtpmidi::sessions::events::event_handling::StreamFaultEvent,
                move |ssrc| {
                    fault_tx.try_send(ssrc).unwrap();
                },
            )
            .await;
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel(8);
        server
            .add_listener(PacketLossEvent, move |event| {
                loss_tx.try_send(event).unwrap();
            })
            .await;
        let mut invitation = vec![255, 255, b'I', b'N', 0, 0, 0, 2, 0, 0, 0, 9, 0, 0, 0, 42];
        invitation.extend_from_slice(b"Peer\0");
        let mut buffer = [0u8; 512];
        control
            .send_to(&invitation, ("127.0.0.1", port))
            .await
            .unwrap();
        control.recv_from(&mut buffer).await.unwrap();
        data.send_to(&invitation, ("127.0.0.1", port + 1))
            .await
            .unwrap();
        data.recv_from(&mut buffer).await.unwrap();
        assert_eq!(server.participants().await.len(), 1);
        let packet = |sequence: u16, body: &[u8]| {
            let mut bytes = vec![0x80, 0x61];
            bytes.extend_from_slice(&sequence.to_be_bytes());
            bytes.extend_from_slice(&(u32::MAX - 5).to_be_bytes());
            bytes.extend_from_slice(&42u32.to_be_bytes());
            bytes.extend_from_slice(body);
            bytes
        };
        for malformed in [&[0x80][..], &[3, 0x90, 60], &[1, 0x90]] {
            data.send_to(&packet(65534, malformed), ("127.0.0.1", port + 1))
                .await
                .unwrap();
        }
        data.send_to(
            &packet(
                65535,
                &[11, 0x90, 60, 100, 10, 0x80, 60, 0, 10, 0x90, 61, 100],
            ),
            ("127.0.0.1", port + 1),
        )
        .await
        .unwrap();
        for _ in 0..3 {
            assert_eq!(fault_rx.recv().await.unwrap(), 42);
        }
        assert_eq!(events.recv().await.unwrap().timestamp, u32::MAX - 5);
        assert_eq!(events.recv().await.unwrap().timestamp, 4);
        assert_eq!(events.recv().await.unwrap().timestamp, 14);
        for sequence in [0, 2, 1, 2, 3] {
            data.send_to(
                &packet(sequence, &[3, 0x80, 61, 0]),
                ("127.0.0.1", port + 1),
            )
            .await
            .unwrap();
        }
        for _ in 0..3 {
            events.recv().await.unwrap();
        }
        assert_eq!(loss_rx.recv().await.unwrap().lost_packets, 1);
        assert!(
            timeout(Duration::from_millis(30), events.recv())
                .await
                .is_err()
        );
        let mut bye = invitation[..16].to_vec();
        bye[2..4].copy_from_slice(b"BY");
        // A matching SSRC alone is insufficient to terminate a session.
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        stranger.send_to(&bye, ("127.0.0.1", port)).await.unwrap();
        assert!(
            timeout(Duration::from_millis(30), left_rx.recv())
                .await
                .is_err()
        );
        control.send_to(&bye, ("127.0.0.1", port)).await.unwrap();
        left_rx.recv().await.unwrap();
        assert!(server.participants().await.is_empty());
        // Reusing an SSRC after reconnect must start a fresh sequence history.
        control
            .send_to(&invitation, ("127.0.0.1", port))
            .await
            .unwrap();
        control.recv_from(&mut buffer).await.unwrap();
        data.send_to(&invitation, ("127.0.0.1", port + 1))
            .await
            .unwrap();
        data.recv_from(&mut buffer).await.unwrap();
        data.send_to(&packet(0, &[3, 0x90, 62, 100]), ("127.0.0.1", port + 1))
            .await
            .unwrap();
        assert_eq!(
            events.recv().await.unwrap().message,
            MidiMessage::NoteOn(Channel::C1, Note::from(62), Value7::from(100))
        );
        assert!(loss_rx.try_recv().is_err());
        // Real-time commands between fragments preserve the assembly.
        for (seq, body) in [
            (1, &[4, 0xF0, 0x7D, 1, 0xF0][..]),
            (2, &[1, 0xF8][..]),
            (3, &[3, 0xF7, 2, 0xF7][..]),
        ] {
            data.send_to(&packet(seq, body), ("127.0.0.1", port + 1))
                .await
                .unwrap();
        }
        assert_eq!(
            events.recv().await.unwrap().message,
            MidiMessage::TimingClock
        );
        assert_eq!(sysex_rx.recv().await.unwrap(), [0x7D, 1, 2]);
        // Cancellation and packet loss must never emit partial SysEx.
        for (seq, body) in [
            (4, &[3, 0xF0, 1, 0xF0][..]),
            (5, &[2, 0xF7, 0xF4][..]),
            (6, &[3, 0xF7, 2, 0xF7][..]),
            (7, &[3, 0xF0, 1, 0xF0][..]),
            (9, &[3, 0xF7, 2, 0xF7][..]),
        ] {
            data.send_to(&packet(seq, body), ("127.0.0.1", port + 1))
                .await
                .unwrap();
        }
        assert_eq!(fault_rx.recv().await.unwrap(), 42);
        assert_eq!(fault_rx.recv().await.unwrap(), 42);
        assert_eq!(loss_rx.recv().await.unwrap().lost_packets, 1);
        assert!(sysex_rx.try_recv().is_err());
        // A corrupt packet also invalidates an assembly even without a gap.
        for (seq, body) in [
            (10, &[3, 0xF0, 1, 0xF0][..]),
            (11, &[3, 0xF7][..]),
            (11, &[3, 0xF7, 2, 0xF7][..]),
        ] {
            data.send_to(&packet(seq, body), ("127.0.0.1", port + 1))
                .await
                .unwrap();
        }
        assert_eq!(fault_rx.recv().await.unwrap(), 42);
        assert_eq!(fault_rx.recv().await.unwrap(), 42);
        assert!(sysex_rx.try_recv().is_err());
        server.stop_gracefully().await;
    })
    .await
    .expect("UDP regression timed out");
}
