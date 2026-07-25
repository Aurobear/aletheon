use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Tracks the "shape" of the cache-relevant prefix across turns.
/// If the shape changes, the cache is invalidated.
#[derive(Debug, Clone)]
pub struct CacheShape {
    /// Hash of the system prompt text
    pub system_hash: u64,
    /// Hash of the tool schemas (sorted deterministically)
    pub tools_hash: u64,
    /// Combined prefix hash
    pub prefix_hash: u64,
    /// Incremented on each compaction (the only deliberate cache-reset point)
    pub rewrite_version: u32,
}

impl CacheShape {
    /// Compute the current cache shape from system prompt and tool names.
    pub fn compute(system_prompt: &str, tool_names: &[&str]) -> Self {
        let system_hash = Self::hash_str(system_prompt);

        // Sort tool names deterministically before hashing
        let mut sorted_tools: Vec<&str> = tool_names.to_vec();
        sorted_tools.sort();
        let tools_str = sorted_tools.join(",");
        let tools_hash = Self::hash_str(&tools_str);

        let mut combined = DefaultHasher::new();
        system_hash.hash(&mut combined);
        tools_hash.hash(&mut combined);
        let prefix_hash = combined.finish();

        Self {
            system_hash,
            tools_hash,
            prefix_hash,
            rewrite_version: 0,
        }
    }

    /// Increment rewrite version (called on compaction).
    pub fn increment_rewrite(&mut self) {
        self.rewrite_version = self.rewrite_version.wrapping_add(1);
    }

    /// Compare with a previous shape and explain any cache miss.
    pub fn compare(&self, prev: &CacheShape) -> CacheComparison {
        if self.prefix_hash == prev.prefix_hash && self.rewrite_version == prev.rewrite_version {
            return CacheComparison::Hit;
        }

        let mut reasons = Vec::new();

        if self.system_hash != prev.system_hash {
            reasons.push(CacheMissReason::SystemChanged);
        }
        if self.tools_hash != prev.tools_hash {
            reasons.push(CacheMissReason::ToolsChanged);
        }
        if self.rewrite_version != prev.rewrite_version {
            reasons.push(CacheMissReason::Compacted);
        }

        CacheComparison::Miss { reasons }
    }

    fn hash_str(s: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }
}

/// Result of comparing two cache shapes.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheComparison {
    /// Cache hit — prefix is identical
    Hit,
    /// Cache miss — with reasons
    Miss { reasons: Vec<CacheMissReason> },
}

/// Why the cache was invalidated.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheMissReason {
    /// System prompt text changed
    SystemChanged,
    /// Tool schemas changed
    ToolsChanged,
    /// Context was compacted (deliberate cache reset)
    Compacted,
}

/// Tracks session-wide cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    pub total_hit_tokens: u64,
    pub total_miss_tokens: u64,
}

impl CacheStats {
    /// Record cache hit/miss from an LLM response.
    pub fn record(&mut self, hit_tokens: u64, miss_tokens: u64) {
        self.total_hit_tokens += hit_tokens;
        self.total_miss_tokens += miss_tokens;
    }

    /// Cache hit rate as a percentage (0.0 - 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.total_hit_tokens + self.total_miss_tokens;
        if total == 0 {
            return 0.0;
        }
        self.total_hit_tokens as f64 / total as f64
    }

    /// Total tokens processed.
    pub fn total_tokens(&self) -> u64 {
        self.total_hit_tokens + self.total_miss_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_deterministic() {
        let s1 = CacheShape::compute("hello", &["a", "b"]);
        let s2 = CacheShape::compute("hello", &["a", "b"]);
        assert_eq!(s1.prefix_hash, s2.prefix_hash);
    }

    #[test]
    fn tool_order_independent() {
        let s1 = CacheShape::compute("sys", &["b", "a"]);
        let s2 = CacheShape::compute("sys", &["a", "b"]);
        assert_eq!(s1.tools_hash, s2.tools_hash);
        assert_eq!(s1.prefix_hash, s2.prefix_hash);
    }

    #[test]
    fn system_change_detected() {
        let s1 = CacheShape::compute("prompt A", &["tool"]);
        let s2 = CacheShape::compute("prompt B", &["tool"]);
        let comp = s2.compare(&s1);
        assert_eq!(
            comp,
            CacheComparison::Miss {
                reasons: vec![CacheMissReason::SystemChanged],
            }
        );
    }

    #[test]
    fn tools_change_detected() {
        let s1 = CacheShape::compute("sys", &["a"]);
        let s2 = CacheShape::compute("sys", &["a", "b"]);
        let comp = s2.compare(&s1);
        assert_eq!(
            comp,
            CacheComparison::Miss {
                reasons: vec![CacheMissReason::ToolsChanged],
            }
        );
    }

    #[test]
    fn compaction_detected() {
        let s1 = CacheShape::compute("sys", &["a"]);
        let mut s2 = CacheShape::compute("sys", &["a"]);
        s2.increment_rewrite();
        let comp = s2.compare(&s1);
        assert_eq!(
            comp,
            CacheComparison::Miss {
                reasons: vec![CacheMissReason::Compacted],
            }
        );
    }

    #[test]
    fn hit_when_identical() {
        let s1 = CacheShape::compute("sys", &["a"]);
        let s2 = CacheShape::compute("sys", &["a"]);
        assert_eq!(s2.compare(&s1), CacheComparison::Hit);
    }

    #[test]
    fn stats_hit_rate() {
        let mut stats = CacheStats::default();
        stats.record(1000, 200);
        assert!((stats.hit_rate() - 0.833).abs() < 0.01);
        assert_eq!(stats.total_tokens(), 1200);
    }

    #[test]
    fn stats_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }
}
