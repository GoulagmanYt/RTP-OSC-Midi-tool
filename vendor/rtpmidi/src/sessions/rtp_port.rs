use std::{ffi::CStr, net::SocketAddr, sync::Arc};

use tokio::net::UdpSocket;
use tracing::{Level, event, instrument};
use zerocopy::network_endian::U32;

use crate::{packets::control_packets::control_packet::ControlPacket, participant::Participant};

pub(super) trait RtpPort {
    fn session_name(&self) -> &CStr;
    fn ssrc(&self) -> U32;
    fn socket(&self) -> &Arc<UdpSocket>;
    fn participant_addr(participant: &Participant) -> SocketAddr;

    #[instrument(skip_all, fields(destination = %destination))]
    async fn send_invitation_acceptance(&self, initiator_token: U32, destination: SocketAddr) {
        let response_packet = ControlPacket::new_acceptance_as_bytes(
            initiator_token,
            self.ssrc(),
            self.session_name(),
        );

        if let Err(e) = self.socket().send_to(&response_packet, destination).await {
            event!(Level::ERROR, "Failed to send invitation response: {}", e);
        } else {
            event!(Level::INFO, "Sent invitation acceptance");
        }
    }

    #[instrument(skip_all, fields(destination = %participant.addr(), participant = participant.name().to_str().unwrap_or("Unknown")))]
    async fn send_termination_packet(&self, participant: &Participant) {
        let termination_packet = ControlPacket::new_termination_as_bytes(
            participant.initiator_token().unwrap_or(U32::ZERO),
            self.ssrc(),
        );
        let addr = Self::participant_addr(participant);
        // BY is best effort over UDP; socket pressure must not hold shutdown.
        match self.socket().try_send_to(&termination_packet, addr) {
            Ok(_) => event!(Level::INFO, "Sent termination packet"),
            Err(e) => event!(Level::WARN, "Failed to send termination packet: {}", e),
        }
    }
}
