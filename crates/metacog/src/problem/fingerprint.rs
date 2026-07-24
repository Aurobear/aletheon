//! Problem fingerprinting — stable problem identity for deduplication.
//!
//! Fingerprints are SHA-256 digests derived from:
//!   domain + subject + category + normalized failure signature + rubric version
//!
//! Semantic similarity may suggest related records, but it cannot automatically
//! merge them without deterministic compatibility checks.

use sha2::{Digest, Sha256};

/// Compute a SHA-256 fingerprint for a problem.
///
/// The input components are concatenated with a null separator and hashed.
/// Changing any component (including rubric version) produces a different fingerprint.
pub fn problem_fingerprint(
    domain: &str,
    subject: &str,
    category: &str,
    failure_signature: &str,
    rubric_version: u32,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    hasher.update(b"\0");
    hasher.update(category.as_bytes());
    hasher.update(b"\0");
    hasher.update(failure_signature.as_bytes());
    hasher.update(b"\0");
    hasher.update(rubric_version.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_produce_identical_fingerprints() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
        let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn different_domain_produces_different_fingerprint() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
        let fp2 = problem_fingerprint("robot", "rustc", "correctness", "type_error", 1);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_subject_produces_different_fingerprint() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
        let fp2 = problem_fingerprint("coding", "clippy", "correctness", "type_error", 1);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_category_produces_different_fingerprint() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
        let fp2 = problem_fingerprint("coding", "rustc", "efficiency", "type_error", 1);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn different_failure_signature_produces_different_fingerprint() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
        let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0499", 1);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn rubric_version_change_produces_different_fingerprint() {
        let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
        let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 2);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_is_stable_hex_string() {
        let fp = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
        assert_eq!(fp.len(), 64); // SHA-256 hex is always 64 chars
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
