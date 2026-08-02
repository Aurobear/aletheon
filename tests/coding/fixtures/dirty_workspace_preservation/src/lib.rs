/// Normalize a user label without changing its internal whitespace.
pub fn normalize_label(value: &str) -> String {
    value.trim_start().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_both_edges() {
        assert_eq!(normalize_label("  alpha beta  "), "alpha beta");
    }
}
