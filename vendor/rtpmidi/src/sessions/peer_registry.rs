use crate::participant::Participant;
use arc_swap::{ArcSwap, Guard};
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::sync::{Mutex, MutexGuard};
use zerocopy::network_endian::U32;

/// Writers serialize session management; packet readers use immutable snapshots.
pub(super) struct PeerRegistry {
    peers: Mutex<HashMap<U32, Participant>>,
    published: ArcSwap<Vec<Participant>>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
            published: ArcSwap::from_pointee(Vec::new()),
        }
    }

    pub fn snapshot(&self) -> Guard<Arc<Vec<Participant>>> {
        self.published.load()
    }

    pub async fn lock(&self) -> PeerWriteGuard<'_> {
        PeerWriteGuard {
            peers: self.peers.lock().await,
            published: &self.published,
            dirty: false,
        }
    }
}

pub(super) struct PeerWriteGuard<'a> {
    peers: MutexGuard<'a, HashMap<U32, Participant>>,
    published: &'a ArcSwap<Vec<Participant>>,
    dirty: bool,
}
impl Deref for PeerWriteGuard<'_> {
    type Target = HashMap<U32, Participant>;
    fn deref(&self) -> &Self::Target {
        &self.peers
    }
}
impl DerefMut for PeerWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty = true;
        &mut self.peers
    }
}
impl Drop for PeerWriteGuard<'_> {
    fn drop(&mut self) {
        if self.dirty {
            // Publish before releasing the writer lock, preserving update order.
            self.published
                .store(Arc::new(self.peers.values().cloned().collect()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readers_keep_consistent_snapshots_while_management_is_locked() {
        let registry = PeerRegistry::new();
        let peer = Participant::new(
            "127.0.0.1:5004".parse().unwrap(),
            true,
            None,
            c"peer",
            U32::new(7),
        );
        registry.lock().await.insert(peer.ssrc(), peer);
        let before = registry.snapshot();
        let mut writer = registry.lock().await;
        writer.clear();
        assert_eq!(registry.snapshot().len(), 1);
        drop(writer);
        assert!(registry.snapshot().is_empty());
        assert_eq!(before.len(), 1);
    }
}
