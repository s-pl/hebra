//! A small sharded map whose entries expire.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const SHARDS: usize = 32;

/// Used to avoid a database round
/// trip per request for token checks and repository visibility; the TTL
/// bounds how long a revoked token keeps working on a replica.
pub struct TtlCache<V> {
    shards: Vec<Mutex<HashMap<String, (V, Instant)>>>,
    max_per_shard: usize,
}

impl<V: Clone> TtlCache<V> {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            max_per_shard: (max_entries / SHARDS).max(16),
        }
    }

    fn shard(&self, key: &str) -> std::sync::MutexGuard<'_, HashMap<String, (V, Instant)>> {
        let i = usize::from(blake3::hash(key.as_bytes()).as_bytes()[0]) % SHARDS;
        self.shards[i]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<V> {
        let shard = self.shard(key);
        shard
            .get(key)
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(v, _)| v.clone())
    }

    pub fn put(&self, key: &str, value: V, ttl: std::time::Duration) {
        let mut shard = self.shard(key);
        if shard.len() >= self.max_per_shard {
            let now = Instant::now();
            shard.retain(|_, (_, until)| now < *until);
            if shard.len() >= self.max_per_shard {
                shard.clear();
            }
        }
        shard.insert(key.to_owned(), (value, Instant::now() + ttl));
    }

    pub fn remove(&self, key: &str) {
        self.shard(key).remove(key);
    }

    pub fn clear(&self) {
        for shard in &self.shards {
            shard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
    }
}
