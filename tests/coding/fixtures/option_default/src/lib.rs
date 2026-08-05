pub fn effective(value: Option<u32>) -> u32 { value.unwrap_or(0) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(effective(None), 30); assert_eq!(effective(Some(7)), 7); }
}
