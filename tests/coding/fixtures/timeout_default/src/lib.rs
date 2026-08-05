pub fn timeout(value: u64) -> u64 { value }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(timeout(0), 30000); assert_eq!(timeout(12), 12); }
}
