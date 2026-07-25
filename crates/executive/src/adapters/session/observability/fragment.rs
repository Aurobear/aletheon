use tracing::debug;

/// Accumulates streaming text fragments from LLM streaming responses.
///
/// Each delta is appended in order. The accumulated result can be
/// retrieved via `content()` or drained via `take()`.
#[derive(Debug, Default)]
pub struct FragmentAccumulator {
    fragments: Vec<String>,
    total_bytes: usize,
}

impl FragmentAccumulator {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a streaming delta fragment.
    pub fn push(&mut self, delta: impl Into<String>) {
        let delta = delta.into();
        self.total_bytes += delta.len();
        debug!(
            len = delta.len(),
            total = self.total_bytes,
            "Fragment pushed"
        );
        self.fragments.push(delta);
    }

    /// Get the concatenated content of all fragments.
    pub fn content(&self) -> String {
        self.fragments.concat()
    }

    /// Drain all fragments and return the concatenated content.
    /// Resets the accumulator.
    pub fn take(&mut self) -> String {
        let result = self.fragments.concat();
        self.fragments.clear();
        self.total_bytes = 0;
        result
    }

    /// Number of fragments accumulated.
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    /// Total bytes accumulated across all fragments.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Whether the accumulator is empty.
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accumulate_fragments() {
        let mut acc = FragmentAccumulator::new();
        assert!(acc.is_empty());

        acc.push("Hello");
        acc.push(", ");
        acc.push("world!");

        assert_eq!(acc.fragment_count(), 3);
        assert_eq!(acc.content(), "Hello, world!");
        assert_eq!(acc.total_bytes(), 13);
    }

    #[test]
    fn test_take_drains() {
        let mut acc = FragmentAccumulator::new();
        acc.push("aaa");
        acc.push("bbb");

        let result = acc.take();
        assert_eq!(result, "aaabbb");
        assert!(acc.is_empty());
        assert_eq!(acc.fragment_count(), 0);
        assert_eq!(acc.total_bytes(), 0);
    }

    #[test]
    fn test_empty_content() {
        let acc = FragmentAccumulator::new();
        assert_eq!(acc.content(), "");
        assert_eq!(acc.total_bytes(), 0);
    }
}
