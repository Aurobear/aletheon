/// Apply the weighting rule described in `SPEC.md`.
pub fn weighted_total(values: &[i32]) -> i32 {
    values.iter().sum::<i32>() * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_example_is_stable() {
        assert_eq!(weighted_total(&[1, 2]), 6);
    }
}
