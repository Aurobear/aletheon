pub fn key(parts: &[&str]) -> String { parts.join("|") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(key(&["b", "a"]), "a|b"); }
}
