/// Normalize a lookup key for storage.
pub fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "-")
}
