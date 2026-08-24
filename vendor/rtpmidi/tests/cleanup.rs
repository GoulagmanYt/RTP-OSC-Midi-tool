mod common;
use common::find_consecutive_ports;

use std::{net::UdpSocket, sync::Arc, time::Duration};

use rtpmidi::sessions::{invite_responder::InviteResponder, rtp_midi_session::RtpMidiSession};

static CLEANUP_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_stop_cleanup() {
    let _guard = CLEANUP_TEST_LOCK.lock().await;
    let (control_port, midi_port) = find_consecutive_ports();

    let ssrc = 0x11111111;
    let session = RtpMidiSession::start(control_port, "Cleanup", ssrc, InviteResponder::Accept)
        .await
        .expect("Failed to start RTP MIDI session");

    session.stop_immediately();

    drop(session);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check if the socket is closed
    let _control_port_socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, control_port))
        .expect("Failed to bind control port");
    let _midi_port_socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, midi_port))
        .expect("Failed to bind MIDI port");
}

#[tokio::test]
async fn test_drop_cleanup() {
    let _guard = CLEANUP_TEST_LOCK.lock().await;
    let (control_port, midi_port) = find_consecutive_ports();

    let ssrc = 0x11111111;
    let session = Arc::new(
        RtpMidiSession::start(control_port, "Cleanup", ssrc, InviteResponder::Accept)
            .await
            .expect("Failed to start RTP MIDI session"),
    );

    drop(session);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check if the socket is closed
    let _control_port_socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, control_port))
        .expect("Failed to bind control port");
    let _midi_port_socket = UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, midi_port))
        .expect("Failed to bind MIDI port");
}
