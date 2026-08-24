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

#[tokio::test]
async fn test_two_session_inter_communication() {
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
