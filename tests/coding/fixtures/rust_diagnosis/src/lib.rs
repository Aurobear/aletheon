pub fn attempts(max_retries: usize) -> usize {
    // max_retries means retries after the initial attempt.
    max_retries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_retries_include_the_initial_attempt() {
        assert_eq!(attempts(3), 4);
    }
}
