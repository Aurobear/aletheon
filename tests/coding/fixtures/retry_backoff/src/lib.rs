pub fn backoff(attempt: u32) -> u64 { attempt as u64 * 100 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(backoff(0), 100); assert_eq!(backoff(3), 400); assert_eq!(backoff(20), 1000); }
}
