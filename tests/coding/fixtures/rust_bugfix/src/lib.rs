/// Return at most `limit` values while preserving input order.
pub fn take_limit<T: Clone>(values: &[T], limit: usize) -> Vec<T> {
    values.iter().take(limit.saturating_sub(1)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_zero_exact_and_oversized_limits() {
        assert_eq!(take_limit(&[1, 2, 3], 0), Vec::<i32>::new());
        assert_eq!(take_limit(&[1, 2, 3], 2), vec![1, 2]);
        assert_eq!(take_limit(&[1, 2, 3], 8), vec![1, 2, 3]);
    }
}
