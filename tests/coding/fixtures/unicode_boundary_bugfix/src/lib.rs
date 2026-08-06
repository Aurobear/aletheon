pub fn truncate_chars(value:&str, limit:usize)->String { value.as_bytes().iter().take(limit).map(|b| char::from(*b)).collect() }
#[cfg(test)] mod tests { use super::*; #[test] fn ascii(){assert_eq!(truncate_chars("abcd",2),"ab");} }
