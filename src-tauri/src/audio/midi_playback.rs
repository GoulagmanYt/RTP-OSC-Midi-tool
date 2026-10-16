//! Allocation-free MIDI source clock. Late events are replayed, rather than
//! shortening every overdue note to zero samples at the next callback.

#[derive(Default)]
pub(super) struct MidiPlaybackClock {
    cursor_us: Option<u64>,
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

    pub(super) fn commit(&mut self, block: &MidiBlockTiming, next_us: Option<u64>) {
        // Capacity spills must not advance the cursor past unsubmitted events.
        let cursor = next_us.map_or(block.end_us, |next| next.min(block.end_us));
        self.cursor_us = (cursor < block.now_us).then_some(cursor.max(block.start_us));
    }

    pub(super) fn reset(&mut self) {
        self.cursor_us = None;
    }
}

pub(super) struct MidiBlockTiming {
    start_us: u64,
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
