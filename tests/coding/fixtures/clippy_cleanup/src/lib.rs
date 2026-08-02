#![deny(clippy::all)]

/// Whether a port is in the supported non-system range.
pub fn is_supported_port(port: u16) -> bool {
    port >= 1024 && port <= 49_151
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_range_boundaries() {
        assert!(!is_supported_port(1023));
        assert!(is_supported_port(1024));
        assert!(is_supported_port(49_151));
        assert!(!is_supported_port(49_152));
    }
}
