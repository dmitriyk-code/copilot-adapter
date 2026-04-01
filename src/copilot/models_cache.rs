use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::types::ModelList;

/// A cached entry holding the model list and the time it was fetched.
struct CacheEntry {
    models: ModelList,
    fetched_at: Instant,
}

/// Thread-safe in-memory cache for the Copilot models list with TTL-based expiration.
///
/// Uses `tokio::sync::RwLock` to allow concurrent reads while ensuring
/// exclusive access for writes.
pub struct ModelsCache {
    inner: RwLock<Option<CacheEntry>>,
    ttl: Duration,
}

impl ModelsCache {
    /// Create a new cache with the given TTL duration.
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(None),
            ttl,
        }
    }

    /// Returns the cached `ModelList` if present and not expired, otherwise `None`.
    pub async fn get(&self) -> Option<ModelList> {
        let guard = self.inner.read().await;
        guard.as_ref().and_then(|entry| {
            if entry.fetched_at.elapsed() < self.ttl {
                Some(entry.models.clone())
            } else {
                None
            }
        })
    }

    /// Store a `ModelList` in the cache with the current timestamp.
    pub async fn set(&self, models: ModelList) {
        let mut guard = self.inner.write().await;
        *guard = Some(CacheEntry {
            models,
            fetched_at: Instant::now(),
        });
    }

    /// Clear the cache, forcing the next `get()` to return `None`.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        *guard = None;
    }
}
