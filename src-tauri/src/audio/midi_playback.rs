//! Allocation-free MIDI source clock. Late events are replayed, rather than
//! shortening every overdue note to zero samples at the next callback.

#[derive(Default)]
pub(super) struct MidiPlaybackClock {
    cursor_us: Option<u64>,
    recovery_started_us: Option<u64>,
}

/// Observation of an admitted block; failed admission does not advance recovery.
#[derive(Default)]
pub(super) struct MidiRecoveryObservation {
    /// Source-clock lag relative to the normal start of this audio block.
    pub(super) lag_us: u64,
    /// Source-clock time from the first recovery block's start through this block's end.
    pub(super) duration_us: u64,
}

impl MidiPlaybackClock {
    pub(super) fn begin_block(
        &self,
        frames: usize,
        sample_rate: u32,
        now_us: u64,
        oldest_us: Option<u64>,
    ) -> MidiBlockTiming {
        let mut block = MidiBlockTiming::new(frames, sample_rate, now_us);
        let start = self.cursor_us.or(oldest_us).unwrap_or(block.start_us);
        if start < block.start_us {
            let period = now_us.saturating_sub(block.start_us).max(1);
            block.start_us = start;
            block.end_us = start.saturating_add(period.saturating_mul(2)).min(now_us);
        }
        block
    }

    pub(super) fn commit(
        &mut self,
        block: &MidiBlockTiming,
        next_us: Option<u64>,
    ) -> MidiRecoveryObservation {
        // Capacity spills must not advance the cursor past unsubmitted events.
        let cursor = next_us.map_or(block.end_us, |next| next.min(block.end_us));
        self.cursor_us = (cursor < block.now_us).then_some(cursor.max(block.start_us));
        let lag_us = block.live_start_us.saturating_sub(block.start_us);
        if lag_us > 0 && self.recovery_started_us.is_none() {
            self.recovery_started_us = Some(block.live_start_us);
        }
        let duration_us = self
            .recovery_started_us
            .map_or(0, |start| block.now_us.saturating_sub(start));
        if self.cursor_us.is_none() {
            self.recovery_started_us = None;
        }
        MidiRecoveryObservation {
            lag_us,
            duration_us,
        }
    }

    pub(super) fn reset(&mut self) {
        self.cursor_us = None;
        self.recovery_started_us = None;
    }
}

pub(super) struct MidiBlockTiming {
    start_us: u64,
    live_start_us: u64,
    end_us: u64,
    now_us: u64,
    frames: u64,
    last_offset: u32,
}

impl MidiBlockTiming {
    pub(super) fn new(frames: usize, sample_rate: u32, now_us: u64) -> Self {
        let frames = frames.max(1) as u64;
        let period_us = frames.saturating_mul(1_000_000) / u64::from(sample_rate.max(1));
        Self {
            start_us: now_us.saturating_sub(period_us.max(1)),
            live_start_us: now_us.saturating_sub(period_us.max(1)),
            end_us: now_us,
            now_us,
            frames,
            last_offset: 0,
        }
    }

    pub(super) fn contains(&self, timestamp_us: u64) -> bool {
        timestamp_us <= self.end_us
    }

    pub(super) fn offset(&mut self, timestamp_us: u64) -> u32 {
        let offset = timestamp_us
            .saturating_sub(self.start_us)
            .saturating_mul(self.frames)
            .checked_div(self.end_us.saturating_sub(self.start_us).max(1))
            .unwrap_or(0)
            .min(self.frames.saturating_sub(1))
            .min(u64::from(u32::MAX)) as u32;
        self.last_offset = self.last_offset.max(offset);
        self.last_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_stalls_measure_recovery_lag_and_duration() {
        for lag_us in [50_000, 100_000, 300_000] {
            let mut clock = MidiPlaybackClock::default();
            let mut max_lag = 0;
            let mut duration = 0;
            for index in 0..100 {
                let block = clock.begin_block(
                    480,
                    48_000,
                    1_000_000 + index * 10_000,
                    (index == 0).then_some(990_000 - lag_us),
                );
                let recovery = clock.commit(&block, None);
                max_lag = max_lag.max(recovery.lag_us);
                duration = duration.max(recovery.duration_us);
                if clock.cursor_us.is_none() {
                    break;
                }
            }
            assert_eq!(max_lag, lag_us);
            assert_eq!(
                duration, lag_us,
                "2x recovery takes one extra block per block of lag"
            );
            println!("simulated lag_us={max_lag}, recovery_duration_us={duration}");
        }
    }

    #[test]
    fn reset_starts_a_new_recovery_episode_and_live_blocks_report_zero() {
        let mut clock = MidiPlaybackClock::default();
        let live = clock.begin_block(480, 48_000, 1_000_000, None);
        assert_eq!(clock.commit(&live, None).duration_us, 0);
        let late = clock.begin_block(480, 48_000, 1_010_000, Some(900_000));
        assert_eq!(clock.commit(&late, None).duration_us, 10_000);
        // An uncommitted (failed admission) block must not advance recovery.
        let _ = clock.begin_block(480, 48_000, 1_500_000, Some(900_000));
        clock.reset();
        let late = clock.begin_block(480, 48_000, 2_000_000, Some(1_900_000));
        let recovery = clock.commit(&late, Some(1_905_000));
        assert_eq!(recovery.duration_us, 10_000);
        assert_eq!(recovery.lag_us, 90_000);
        let spilled = clock.begin_block(480, 48_000, 2_010_000, None);
        assert_eq!(clock.commit(&spilled, None).duration_us, 20_000);
    }

    #[test]
    fn telemetry_keeps_session_maxima_across_recovery_resets() {
        use super::super::runtime_state::AudioTelemetry;
        use std::sync::atomic::Ordering;
        let telemetry = AudioTelemetry::new();
        let mut clock = MidiPlaybackClock::default();
        for (now, oldest) in [(1_000_000, 900_000), (1_010_000, 0)] {
            let block = clock.begin_block(480, 48_000, now, Some(oldest));
            telemetry.record_midi_recovery(clock.commit(&block, None));
        }
        clock.reset();
        let live = clock.begin_block(480, 48_000, 2_000_000, None);
        telemetry.record_midi_recovery(clock.commit(&live, None));
        assert_eq!(
            telemetry.midi_recovery_max_lag_us.load(Ordering::Relaxed),
            90_000
        );
        assert_eq!(
            telemetry
                .midi_recovery_max_duration_us
                .load(Ordering::Relaxed),
            20_000
        );
    }

    #[test]
    fn time_and_frame_boundaries_saturate_without_wrapping() {
        let mut near_origin = MidiBlockTiming::new(480, 48_000, 5);
        assert_eq!(near_origin.offset(0), 0);
        assert_eq!(near_origin.offset(5), 479);
        assert!(!near_origin.contains(6));

        let mut near_limit = MidiBlockTiming::new(480, 48_000, u64::MAX);
        assert_eq!(near_limit.offset(u64::MAX - 5_000), 240);
        assert_eq!(near_limit.offset(u64::MAX), 479);

        let mut oversized = MidiBlockTiming::new(usize::MAX, 0, u64::MAX);
        assert!(oversized.contains(u64::MAX));
        assert!(u64::from(oversized.offset(u64::MAX)) < oversized.frames);
        assert_eq!(MidiBlockTiming::new(0, 0, 0).offset(u64::MAX), 0);
    }

    #[test]
    fn late_note_retains_duration_and_clock_catches_up() {
        let mut clock = MidiPlaybackClock::default();
        let mut rendered = Vec::new();
        let mut pending = std::collections::VecDeque::from([900_000, 950_000, 1_005_000]);
        for index in 0..12 {
            let mut block = clock.begin_block(
                480,
                48_000,
                1_000_000 + index * 10_000,
                pending.front().copied(),
            );
            assert!(block.end_us - block.start_us <= 20_000);
            while pending.front().is_some_and(|stamp| block.contains(*stamp)) {
                let stamp = pending.pop_front().unwrap();
                rendered.push((stamp, index * 480 + u64::from(block.offset(stamp))));
            }
            clock.commit(&block, pending.front().copied());
        }
        assert!(pending.is_empty());
        assert_eq!(
            rendered[1].1 - rendered[0].1,
            1_200,
            "50 ms note is replayed at 2x, never at zero duration"
        );
        assert!(rendered[2].1 > rendered[1].1);
        assert!(
            clock.cursor_us.is_none(),
            "recovery must return to live timing"
        );
    }

    #[test]
    fn capacity_spill_and_failed_admission_do_not_skip_source_time() {
        let mut clock = MidiPlaybackClock::default();
        let first = clock.begin_block(480, 48_000, 1_000_000, Some(900_000));
        let retry = clock.begin_block(480, 48_000, 1_010_000, Some(900_000));
        assert_eq!(first.start_us, retry.start_us); // No commit on failed admission.
        clock.commit(&retry, Some(905_000));
        let next = clock.begin_block(480, 48_000, 1_020_000, Some(905_000));
        assert_eq!(next.start_us, 905_000);
        assert!(next.contains(910_000));
    }

    #[test]
    fn reset_discards_recovery_latency_and_repeated_stalls_remain_bounded() {
        let mut clock = MidiPlaybackClock::default();
        let block = clock.begin_block(480, 48_000, 1_000_000, Some(900_000));
        clock.commit(&block, None);
        let stalled = clock.begin_block(960, 48_000, 1_200_000, Some(950_000));
        assert_eq!(stalled.end_us - stalled.start_us, 40_000);
        clock.reset();
        let mut normal = clock.begin_block(480, 48_000, 1_210_000, Some(1_205_000));
        assert_eq!(normal.offset(1_205_000), 240);
        assert!(!normal.contains(1_215_000));
    }
}
