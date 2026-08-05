pub fn sum(values: &[u32]) -> u32 { values.iter().copied().sum() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(sum(&[u32::MAX, 1]), u32::MAX); }
}
