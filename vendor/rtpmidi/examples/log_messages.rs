#[cfg(feature = "examples")]
#[tokio::main]
async fn main() {
    use rtpmidi::sessions::{
        events::event_handling::{MidiMessageEvent, SysExPacketEvent},
        invite_responder::InviteResponder,
        rtp_midi_session::RtpMidiSession,
    };
    use tracing::{Level, event};
    use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let session = RtpMidiSession::start(5004, "My Session", 12345, InviteResponder::Accept)
        .await
        .expect("Failed to start RTP-MIDI session");

    session
        .add_listener(MidiMessageEvent, move |data| {
            event!(Level::INFO, "Received command: {:?}", data);
        })
        .await;

    session
        .add_listener(SysExPacketEvent, |data| {
            event!(Level::INFO, "Received SysEx packet: {:?}", data);
        })
        .await;

    // tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("Failed to set up Ctrl+C signal handler")
        .recv()
        .await;
    println!("Ctrl+C received, stopping session...");
    event!(Level::INFO, "Stopping RTP-MIDI session gracefully");
    session.stop_gracefully().await;
}

#[cfg(not(feature = "examples"))]
fn main() {
    println!("This example requires the 'examples' feature to be enabled.");
}
