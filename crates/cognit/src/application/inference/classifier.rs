/// Complexity level for intent classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Complexity {
    Simple,
    Medium,
    Complex,
}

/// Rule-based intent classifier (Phase 1).
pub struct IntentClassifier {
    simple_keywords: Vec<String>,
    complex_keywords: Vec<String>,
    token_threshold: usize,
}

impl Default for IntentClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl IntentClassifier {
    pub fn new() -> Self {
        Self {
            simple_keywords: vec![
                "read".into(),
                "show".into(),
                "list".into(),
                "check".into(),
                "status".into(),
                "echo".into(),
                "cat".into(),
                "ls".into(),
            ],
            complex_keywords: vec![
                "analyze".into(),
                "debug".into(),
                "refactor".into(),
                "optimize".into(),
                "design".into(),
                "architect".into(),
                "migrate".into(),
                "deploy".into(),
            ],
            token_threshold: 500,
        }
    }

    /// Classify message complexity.
    pub fn classify(&self, message: &str) -> Complexity {
        let lower = message.to_lowercase();
        let word_count = message.split_whitespace().count();

        // Check for simple patterns
        let simple_score = self
            .simple_keywords
            .iter()
            .filter(|k| lower.contains(k.as_str()))
            .count();

        // Check for complex patterns
        let complex_score = self
            .complex_keywords
            .iter()
            .filter(|k| lower.contains(k.as_str()))
            .count();

        // Decision logic
        if complex_score >= 2 || word_count > self.token_threshold {
            Complexity::Complex
        } else if simple_score >= 1 && complex_score == 0 && word_count < 50 {
            Complexity::Simple
        } else {
            Complexity::Medium
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_classification() {
        let classifier = IntentClassifier::new();
        assert_eq!(
            classifier.classify("read file /etc/hostname"),
            Complexity::Simple
        );
        assert_eq!(
            classifier.classify("show system status"),
            Complexity::Simple
        );
        assert_eq!(classifier.classify("list processes"), Complexity::Simple);
    }

    #[test]
    fn test_complex_classification() {
        let classifier = IntentClassifier::new();
        assert_eq!(
            classifier
                .classify("analyze the performance bottleneck and optimize the database queries"),
            Complexity::Complex
        );
        assert_eq!(
            classifier.classify("debug the failing test and refactor the authentication module"),
            Complexity::Complex
        );
    }

    #[test]
    fn test_medium_classification() {
        let classifier = IntentClassifier::new();
        assert_eq!(
            classifier.classify("fetch the user data from the API"),
            Complexity::Medium
        );
    }
}
