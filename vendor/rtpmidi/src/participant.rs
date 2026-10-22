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
    identity: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct ClockMapping {
    remote: u64,
    local: Instant,
    rate_ppb: i64,
    round_trip: Duration,
    frequency_anchor: ClockSample,
}

#[derive(Debug, Clone, PartialEq)]
struct ClockSample {
    remote: u64,
    local: Instant,
    round_trip: Duration,
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
            identity: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    pub(crate) fn identity(&self) -> u64 {
        self.identity
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

    pub(super) fn synchronize(&mut self, remote: u64, local: Instant, round_trip: Duration) {
        // Startup CK bursts measure packet/scheduler jitter, not oscillator
        // frequency. Keep a separate, low-RTT anchor over a >=10s baseline.
        // A stalled exchange must not shift the phase or extrapolate tens of
        // milliseconds of artificial future note deadlines.
        const MIN_RATE_BASELINE_NS: i128 = 10_000_000_000;
        const MAX_RATE_PPB: i128 = 1_000_000;
        const RTT_TOLERANCE: Duration = Duration::from_micros(100);
        const PHASE_MAX_AGE: Duration = Duration::from_secs(30);
        let sample = ClockSample {
            remote,
            local,
            round_trip,
        };
        let (mut rate_ppb, mut anchor, protected_rtt) = match self.clock.as_ref() {
            Some(previous) => {
                if remote <= previous.remote || local < previous.local {
                    return;
                }
                let noisier = round_trip > previous.round_trip + RTT_TOLERANCE;
                if noisier && local.duration_since(previous.local) < PHASE_MAX_AGE {
                    return;
                }
                let anchor = if noisier {
                    sample.clone()
                } else {
                    previous.frequency_anchor.clone()
                };
                let protected_rtt = if noisier {
                    round_trip
                } else {
                    previous.round_trip.min(round_trip)
                };
                (previous.rate_ppb, anchor, protected_rtt)
            }
            None => (0, sample.clone(), round_trip),
        };
        if round_trip < anchor.round_trip {
            // A cleaner exchange supersedes an uncertain initial anchor.
            anchor = sample.clone();
        }
        let remote_ns = i128::from(remote.saturating_sub(anchor.remote)) * 100_000;
        if remote_ns >= MIN_RATE_BASELINE_NS && local >= anchor.local {
            let local_ns = local.duration_since(anchor.local).as_nanos() as i128;
            let fit = (local_ns - remote_ns) * 1_000_000_000 / remote_ns;
            // Each midpoint is uncertain by up to half its RTT. Do not turn
            // that uncertainty into a frequency correction. Reject implausible
            // fits instead of saturating noise at the maximum allowed drift.
            let uncertainty = ((round_trip + anchor.round_trip).as_nanos() as i128 / 2 + 100_000)
                * 1_000_000_000
                / remote_ns;
            if fit.abs() <= MAX_RATE_PPB && (fit - i128::from(rate_ppb)).abs() > uncertainty {
                rate_ppb = fit as i64;
            }
        }
        tracing::debug!(
            rate_ppb,
            round_trip_us = round_trip.as_micros() as u64,
            rate_baseline_ms = (remote_ns / 1_000_000) as u64,
            "Updated RTP clock mapping"
        );
        self.clock = Some(ClockMapping {
            remote,
            local,
            rate_ppb,
            round_trip: protected_rtt,
            frequency_anchor: anchor,
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
        peer.synchronize(remote, now, Duration::ZERO);
        assert_eq!(peer.deadline(4).unwrap(), now + Duration::from_millis(1));
        assert_eq!(
            peer.deadline((remote - 10) as u32).unwrap(),
            now - Duration::from_millis(1)
        );
        peer.synchronize(
            remote + 100_000,
            now + Duration::from_micros(10_001_000),
            Duration::ZERO,
        );
        let deadline = peer.deadline((remote + 110_000) as u32).unwrap();
        assert_eq!(deadline, now + Duration::from_micros(11_001_100));
    }

    fn peer() -> Participant {
        Participant::new(
            "127.0.0.1:5004".parse().unwrap(),
            true,
            Some(U32::new(1)),
            c"clock-test",
            U32::new(2),
        )
    }

    #[test]
    fn short_noisy_sync_does_not_extrapolate_network_jitter_into_clock_drift() {
        let mut peer = peer();
        let now = Instant::now();
        peer.synchronize(0, now, Duration::ZERO);
        peer.synchronize(10_000, now + Duration::from_millis(1002), Duration::ZERO);
        let deadline = peer.deadline(300_000).unwrap();
        assert!(
            deadline <= now + Duration::from_millis(30_003),
            "2ms of midpoint jitter must not become 31ms of artificial latency"
        );
    }

    #[test]
    fn short_or_reversed_sync_preserves_an_established_clock_rate() {
        let mut peer = peer();
        let now = Instant::now();
        peer.synchronize(0, now, Duration::ZERO);
        peer.synchronize(
            100_000,
            now + Duration::from_micros(10_001_000),
            Duration::ZERO,
        );
        peer.synchronize(
            100_100,
            now + Duration::from_micros(10_011_001),
            Duration::ZERO,
        );
        let expected = now + Duration::from_micros(20_002_000);
        assert_eq!(
            peer.deadline(200_000),
            Some(expected),
            "short sample must retain the known 100ppm drift"
        );
        peer.synchronize(99_999, now + Duration::from_secs(10), Duration::ZERO);
        assert_eq!(
            peer.deadline(200_000),
            Some(expected),
            "reversed sample must not rewind the clock model"
        );
    }

    #[test]
    fn stalled_high_rtt_sync_does_not_replace_a_recent_clean_anchor() {
        let mut peer = peer();
        let now = Instant::now();
        peer.synchronize(0, now, Duration::from_millis(2));
        peer.synchronize(
            10_000,
            now + Duration::from_millis(1010),
            Duration::from_millis(22),
        );
        assert_eq!(
            peer.deadline(300_000),
            Some(now + Duration::from_secs(30)),
            "CPU-stalled clock exchange must not delay subsequent notes"
        );
    }

    #[test]
    fn gradually_worsening_rtt_cannot_walk_away_from_the_clean_phase_anchor() {
        let mut peer = peer();
        let now = Instant::now();
        peer.synchronize(0, now, Duration::from_millis(2));
        for index in 1..=20 {
            peer.synchronize(
                index * 10_000,
                now + Duration::from_secs(index) + Duration::from_micros(index * 50),
                Duration::from_micros(2000 + index * 100),
            );
        }
        assert!(
            peer.deadline(300_000).unwrap()
                <= now + Duration::from_secs(30) + Duration::from_micros(100),
            "small successive RTT increases must be compared to the retained clean anchor"
        );
    }
}
