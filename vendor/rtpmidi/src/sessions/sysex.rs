use std::time::{Duration, Instant};

pub(crate) const MAX_SYSEX_BYTES: usize = 65_536;
const FRAGMENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Storage is reserved at participant creation, never grown by packet input.
#[derive(Debug, PartialEq)]
pub(crate) struct SysExAssembly {
    bytes: Vec<u8>,
    updated: Option<Instant>,
}

impl Clone for SysExAssembly {
    fn clone(&self) -> Self {
        let mut copy = Self::new();
        copy.bytes.extend_from_slice(&self.bytes);
        copy.updated = self.updated;
        copy
    }
}

impl SysExAssembly {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_SYSEX_BYTES),
            updated: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_active(&self) -> bool {
        self.updated.is_some()
    }

    pub(crate) fn prefix(&self, now: Instant) -> Option<&[u8]> {
        self.updated
            .filter(|last| now.duration_since(*last) < FRAGMENT_TIMEOUT)
            .map(|_| self.bytes.as_slice())
    }

    pub(crate) fn clear(&mut self) {
        self.bytes.clear();
        self.updated = None;
    }

    pub(crate) fn segment(
        &mut self,
        head: u8,
        data: &[u8],
        tail: u8,
        now: Instant,
    ) -> Result<Option<&[u8]>, ()> {
        if self
            .updated
            .is_some_and(|last| now.duration_since(last) >= FRAGMENT_TIMEOUT)
        {
            self.clear();
        }
        if head == 0xF0 {
            self.clear();
        } else if self.updated.is_none() {
            return Err(());
        }
        if tail == 0xF4 {
            self.clear();
            return Ok(None);
        }
        if data.len() > MAX_SYSEX_BYTES - self.bytes.len() {
            self.clear();
            return Err(());
        }
        self.bytes.extend_from_slice(data);
        if matches!(tail, 0xF7 | 0xF5) {
            self.updated = None;
            Ok(Some(&self.bytes))
        } else {
            self.updated = Some(now);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembles_cancels_expires_and_rejects_oversize_without_growing() {
        let now = Instant::now();
        let mut state = SysExAssembly::new();
        let allocation = state.bytes.as_ptr();
        assert_eq!(state.segment(0xF0, &[1, 2], 0xF0, now), Ok(None));
        assert_eq!(state.segment(0xF7, &[3], 0xF0, now), Ok(None));
        assert_eq!(
            state.segment(0xF7, &[], 0xF7, now),
            Ok(Some(&[1, 2, 3][..]))
        );
        assert_eq!(state.segment(0xF7, &[4], 0xF7, now), Err(()));
        state.segment(0xF0, &[1], 0xF0, now).unwrap();
        assert_eq!(state.segment(0xF7, &[], 0xF4, now), Ok(None));
        assert_eq!(state.segment(0xF7, &[2], 0xF7, now), Err(()));
        state.segment(0xF0, &[1], 0xF0, now).unwrap();
        assert_eq!(
            state.segment(0xF7, &[2], 0xF7, now + FRAGMENT_TIMEOUT),
            Err(())
        );
        state
            .segment(0xF0, &vec![1; MAX_SYSEX_BYTES], 0xF0, now)
            .unwrap();
        assert_eq!(state.segment(0xF7, &[2], 0xF7, now), Err(()));
        assert_eq!(state.bytes.as_ptr(), allocation);
        assert!(state.bytes.is_empty());
    }
}
