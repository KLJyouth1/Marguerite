use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_TTL_SECS: u64 = 60 * 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheValue {
    pub value: String,
    pub input: String,
    pub inserted_at: u64,
    pub expires_at: u64,
}

type Scorer = Arc<dyn Fn(&str, &str) -> f64 + Send + Sync>;

#[derive(Default)]
struct Inner {
    entries: HashMap<String, CacheValue>,
    stats: HitStats,
}

/// Deterministic, namespace-isolated in-memory cache for prompt results.
///
/// Exact fingerprints are preferred; an optional injected scorer can provide
/// conservative semantic matches without adding a vector-store dependency.
pub struct HitCache {
    namespace: String,
    ttl_secs: u64,
    inner: Mutex<Inner>,
    scorer: Option<Scorer>,
    threshold: f64,
}

impl HitCache {
    pub fn new(namespace: &str) -> Result<Self, String> {
        Self::with_options(namespace, DEFAULT_TTL_SECS, None, 1.0)
    }

    pub fn with_clock(namespace: &str, _now: u64) -> Result<Self, String> {
        // The clock argument is retained as a test-friendly constructor contract;
        // callers that need deterministic times should use put_at/get_at.
        Self::new(namespace)
    }

    pub fn with_scorer<F>(namespace: &str, threshold: f64, scorer: F) -> Result<Self, String>
    where
        F: Fn(&str, &str) -> f64 + Send + Sync + 'static,
    {
        Self::with_options(
            namespace,
            DEFAULT_TTL_SECS,
            Some(Arc::new(scorer)),
            threshold,
        )
    }

    fn with_options(
        namespace: &str,
        ttl_secs: u64,
        scorer: Option<Scorer>,
        threshold: f64,
    ) -> Result<Self, String> {
        let namespace = namespace.trim();
        if namespace.is_empty() || namespace.contains(':') {
            return Err("cache namespace must be non-empty and colon-free".into());
        }
        if !(0.0..=1.0).contains(&threshold) {
            return Err("cache scorer threshold must be between 0 and 1".into());
        }
        Ok(Self {
            namespace: namespace.to_owned(),
            ttl_secs: ttl_secs.max(1),
            inner: Mutex::new(Inner::default()),
            scorer,
            threshold,
        })
    }

    pub fn key_for(namespace: &str, input: &str) -> Result<String, String> {
        let namespace = namespace.trim();
        if namespace.is_empty() || namespace.contains(':') || input.trim().is_empty() {
            return Err("cache key input must be non-empty and compatible".into());
        }
        Ok(format!("{}:{:016x}", namespace, fingerprint(input)))
    }

    pub fn fingerprint(input: &str) -> Result<u64, String> {
        if input.trim().is_empty() {
            return Err("cache fingerprint input must be non-empty".into());
        }
        Ok(fingerprint(input))
    }

    pub fn put(&self, input: &str, value: String) -> Result<(), String> {
        self.put_at(input, value, now_secs(), self.ttl_secs)
    }

    pub fn put_at(
        &self,
        input: &str,
        value: String,
        now: u64,
        ttl_secs: u64,
    ) -> Result<(), String> {
        let key = Self::key_for(&self.namespace, input)?;
        self.put_key_at_with_input(&key, input, value, now, ttl_secs)
    }

    pub fn put_key(&self, key: &str, value: String) -> Result<(), String> {
        self.put_key_at_with_input(key, key, value, now_secs(), self.ttl_secs)
    }

    fn put_key_at_with_input(
        &self,
        key: &str,
        input: &str,
        value: String,
        now: u64,
        ttl_secs: u64,
    ) -> Result<(), String> {
        self.validate_key(key)?;
        if value.is_empty() {
            return Err("cache value must be non-empty".into());
        }
        let expires_at = now.saturating_add(ttl_secs.max(1));
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .insert(
                key.to_owned(),
                CacheValue {
                    value,
                    input: input.to_owned(),
                    inserted_at: now,
                    expires_at,
                },
            );
        Ok(())
    }

    pub fn get(&self, input: &str) -> Result<Option<String>, String> {
        let key = Self::key_for(&self.namespace, input)?;
        self.get_key_at(&key, input, now_secs())
    }

    pub fn get_at(&self, input: &str, now: u64) -> Result<Option<String>, String> {
        let key = Self::key_for(&self.namespace, input)?;
        self.get_key_at(&key, input, now)
    }

    pub fn get_key(&self, key: &str) -> Result<Option<String>, String> {
        self.get_key_at(key, "", now_secs())
    }

    fn get_key_at(&self, key: &str, query: &str, now: u64) -> Result<Option<String>, String> {
        self.validate_key(key)?;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.stats.lookups += 1;
        if let Some(entry) = inner.entries.get(key).cloned() {
            if now < entry.expires_at {
                inner.stats.hits += 1;
                return Ok(Some(entry.value));
            }
            inner.entries.remove(key);
        }
        if let Some(scorer) = &self.scorer {
            if !query.is_empty() {
                let match_key = inner.entries.iter().find_map(|(_candidate_key, entry)| {
                    if now < entry.expires_at {
                        let score = scorer(query, &entry.input);
                        (score >= self.threshold).then(|| entry.value.clone())
                    } else {
                        None
                    }
                });
                if match_key.is_some() {
                    inner.stats.hits += 1;
                    return Ok(match_key);
                }
            }
        }
        inner.stats.misses += 1;
        Ok(None)
    }

    pub fn stats(&self) -> HitStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    fn validate_key(&self, key: &str) -> Result<(), String> {
        let prefix = format!("{}:", self.namespace);
        if key.is_empty() || !key.starts_with(&prefix) || key.len() <= prefix.len() {
            return Err("cache key is incompatible with namespace".into());
        }
        Ok(())
    }
}

fn fingerprint(input: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.trim().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_exact_hit() {
        let cache = HitCache::new("prompt").unwrap();
        cache.put("same request", "answer".into()).unwrap();
        assert_eq!(
            cache.get("same request").unwrap().as_deref(),
            Some("answer")
        );
    }

    #[test]
    fn expired_entry_is_not_a_hit() {
        let cache = HitCache::with_clock("prompt", 1000).unwrap();
        cache.put_at("request", "answer".into(), 1000, 1).unwrap();
        assert!(cache.get_at("request", 1000).unwrap().is_some());
        assert!(cache.get_at("request", 1001).unwrap().is_none());
    }

    #[test]
    fn namespaces_are_isolated() {
        let first = HitCache::new("one").unwrap();
        let second = HitCache::new("two").unwrap();
        let key = HitCache::key_for("one", "request").unwrap();
        first.put_key(&key, "answer".into()).unwrap();
        assert!(second.get_key(&key).is_err());
        assert!(second.get("request").unwrap().is_none());
    }

    #[test]
    fn hit_and_miss_statistics_are_recorded() {
        let cache = HitCache::new("prompt").unwrap();
        cache.put("hit", "answer".into()).unwrap();
        assert!(cache.get("hit").unwrap().is_some());
        assert!(cache.get("miss").unwrap().is_none());
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn semantic_scorer_can_match_similar_input() {
        let cache = HitCache::with_scorer("prompt", 0.8, |query, candidate| {
            if query.contains("weather") && candidate.contains("forecast") {
                0.9
            } else {
                0.0
            }
        })
        .unwrap();
        cache.put("today forecast", "sunny".into()).unwrap();
        assert_eq!(
            cache.get("weather today").unwrap().as_deref(),
            Some("sunny")
        );
    }

    #[test]
    fn empty_and_incompatible_keys_are_rejected() {
        assert!(HitCache::new("").is_err());
        let cache = HitCache::new("prompt").unwrap();
        assert!(cache.put_key("other:key", "value".into()).is_err());
        assert!(cache.get_key("").is_err());
    }
}
