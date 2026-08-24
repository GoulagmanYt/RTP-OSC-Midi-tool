# rtpmidi
[![Crates.io Version](https://img.shields.io/crates/v/rtpmidi)](https://crates.io/crates/rtpmidi)
![Crates.io License](https://img.shields.io/crates/l/rtpmidi)
![Codecov](https://img.shields.io/codecov/c/github/iKadmium/rtp-midi-rs)

This provides functions for working with RTP-MIDI in Rust.

## Usage

```rs
let port = 5004_u16;
let ssrc = 123456_u32;
let session = RtpMidiSession::start(port, "My Session", ssrc, InviteResponder::Accept); // you can choose to accept all invitations, none, or supply a custom handler

// Wait for midi commands
session
    .add_listener(RtpMidiEventType::MidiPacket, move |data| {
        for command in data.commands() {
            event!(Level::INFO, "Received command: {:?}", command);
        }
    })
    .await;

// invite another participant to the session
let addr = SocketAddr::new("192.168.0.1".parse().unwrap(), 5006);
let _ = invite_server.invite_participant(addr).await.unwrap();

// send MIDI commands
let command = MidiCommand::NoteOn {
    channel: 1,
    key: 64,
    velocity: 127,
};

session.send_midi(command).await.unwrap();
```

See the Examples directory for more examples.

## Installation

```cargo add rtpmidi```

## Status

Supported:
* Responding to invitations
* Inviting others
* Advertising via MDNS / Bonjour (optional - enable the 'mdns' feature for this)
* SysEx

Not supported:
* Recovery journal
