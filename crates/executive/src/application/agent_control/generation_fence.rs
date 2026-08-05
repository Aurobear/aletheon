//! Daemon-generation fencing for late child terminal receipts.

#[derive(Debug, Clone, Default)]
pub(crate) struct GenerationFence(Option<String>);

impl GenerationFence {
    pub(crate) fn bind(generation: impl Into<String>) -> Self {
        Self(Some(generation.into()))
    }

    pub(crate) fn rejection<'a>(&'a self, received: &'a str) -> Option<(&'a str, &'a str)> {
        let expected = self.0.as_deref()?;
        (expected != received).then_some((expected, received))
    }
}

#[cfg(test)]
mod tests {
    use super::GenerationFence;

    #[test]
    fn rejects_late_generation_and_allows_unbound_compatibility() {
        let fence = GenerationFence::bind("current");
        assert_eq!(fence.rejection("old"), Some(("current", "old")));
        assert_eq!(fence.rejection("current"), None);
        assert_eq!(GenerationFence::default().rejection("old"), None);
    }
}
