pub fn window(values: &[i32], start: usize, end: usize) -> Vec<i32> { values.get(start..end).unwrap_or(&[]).to_vec() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(window(&[1,2,3,4], 1, 3), vec![2,3,4]); }
}
