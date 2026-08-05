pub fn parse_fields(input: &str) -> Result<Vec<String>, String> { Ok(input.split(',').map(|s| s.to_string()).collect()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(parse_fields("a, b,a").unwrap(), vec!["a", "b", "a"]); assert!(parse_fields("a,,b").is_err()); }
}
