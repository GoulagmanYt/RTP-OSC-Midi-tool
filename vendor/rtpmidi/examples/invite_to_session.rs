#[cfg(feature = "examples")]
#[tokio::main]
async fn main() {
    use rtpmidi::sessions::{invite_responder::InviteResponder, rtp_midi_session::RtpMidiSession};
    use std::net::SocketAddr;
    use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let session = RtpMidiSession::start(
        5004,
        "My Session",
        54321,
        InviteResponder::new(|_packet, name, _addr| name.to_str().unwrap() == "Bob's jam session"),
    )
    .await
    .expect("Failed to start RTP MIDI session");

    let addr = SocketAddr::new("192.168.0.28".parse().unwrap(), 5006);
    session.invite_participant(addr).await;

    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl+C");
    session.stop_gracefully().await;
}

#[cfg(not(feature = "examples"))]
fn main() {
    println!("This example requires the 'examples' feature to be enabled.");
}
