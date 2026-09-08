use std::{
    ffi::{CStr, CString},
    fmt::Display,
    net::SocketAddr,
    time::{Duration, Instant},
};

use zerocopy::network_endian::U32;

#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    ctrl_addr: SocketAddr,
    initiator_token: Option<U32>,
    last_clock_sync: Instant,
    name: CString,
    invited_by_us: bool,
    ssrc: U32,
    clock: Option<ClockMapping>,
    pub(crate) expected_sequence: Option<u16>,
    pub(crate) active_notes: [u128; 16],
    pub(crate) sysex: Option<crate::sessions::sysex::SysExAssembly>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClockMapping {
    remote: u64,
    local: Instant,
    rate_ppb: i64,
}

impl Participant {
    pub fn new(
        ctrl_addr: SocketAddr,
        invited_by_us: bool,
        initiator_token: Option<U32>,
        name: &CStr,
        ssrc: U32,
    ) -> Self {
        Participant {
            ctrl_addr,
            initiator_token,
            name: name.to_owned(),
            last_clock_sync: Instant::now(),
            invited_by_us,
            ssrc,
            clock: None,
            expected_sequence: None,
            active_notes: [0; 16],
            sysex: Some(crate::sessions::sysex::SysExAssembly::new()),
        }
    }

    pub(super) fn midi_port_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ctrl_addr.ip(), self.ctrl_addr.port() + 1)
    }

    pub(super) fn last_clock_sync(&self) -> Instant {
        self.last_clock_sync
    }

    pub(super) fn received_clock_sync(&mut self) {
        self.last_clock_sync = Instant::now();
    }

    pub(super) fn synchronize(&mut self, remote: u64, local: Instant) {
        let rate_ppb = self
            .clock
            .as_ref()
            .and_then(|previous| {
                let remote_ns = i128::from(remote.checked_sub(previous.remote)?) * 100_000;
                if remote_ns < 100_000_000 {
                    return None;
                }
                let local_ns = local.checked_duration_since(previous.local)?.as_nanos() as i128;
                Some(
                    ((local_ns - remote_ns) * 1_000_000_000 / remote_ns)
                        .clamp(-1_000_000, 1_000_000) as i64,
                )
            })
            .unwrap_or(0);
        self.clock = Some(ClockMapping {
            remote,
            local,
            rate_ppb,
        });
    }

    pub(super) fn deadline(&self, timestamp: u32) -> Option<Instant> {
        let clock = self.clock.as_ref()?;
        let ticks = i128::from(timestamp.wrapping_sub(clock.remote as u32) as i32);
        let ns = ticks * 100_000 * (1_000_000_000 + i128::from(clock.rate_ppb)) / 1_000_000_000;
        let duration = Duration::from_nanos(ns.unsigned_abs().try_into().ok()?);
        if ns >= 0 {
            clock.local.checked_add(duration)
        } else {
            clock.local.checked_sub(duration)
        }
    }

    pub(super) fn is_invited_by_us(&self) -> bool {
        self.invited_by_us
    }

    pub(super) fn initiator_token(&self) -> Option<U32> {
        self.initiator_token
    }

    pub fn name(&self) -> &CStr {
        &self.name
    }

    pub fn addr(&self) -> SocketAddr {
        self.ctrl_addr
    }

    pub fn ssrc(&self) -> U32 {
        self.ssrc
    }
}

impl Display for Participant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Participant {{ name: {}, addr: {}, ssrc: {} }}",
            self.name.to_str().unwrap_or("Unknown"),
            self.ctrl_addr,
            self.ssrc.get()
        )
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn maps_offset_rollover_and_clock_rate_without_float_rounding() {
        let mut peer = Participant::new(
            "127.0.0.1:5004".parse().unwrap(),
            true,
            Some(U32::new(1)),
            c"test",
            U32::new(2),
        );
        let now = Instant::now();
        let remote = u64::from(u32::MAX) - 5;
        peer.synchronize(remote, now);
        assert_eq!(peer.deadline(4).unwrap(), now + Duration::from_millis(1));
        assert_eq!(
            peer.deadline((remote - 10) as u32).unwrap(),
            now - Duration::from_millis(1)
        );
        peer.synchronize(remote + 100_000, now + Duration::from_micros(10_001_000));
        let deadline = peer.deadline((remote + 110_000) as u32).unwrap();
        assert_eq!(deadline, now + Duration::from_micros(11_001_100));
    }
}
