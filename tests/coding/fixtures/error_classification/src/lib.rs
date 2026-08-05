pub fn classify(input: &str) -> &'static str { if input.is_empty() { "transient" } else { "ok" } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(classify(""), "invalid"); assert_eq!(classify("x"), "ok"); }
}
