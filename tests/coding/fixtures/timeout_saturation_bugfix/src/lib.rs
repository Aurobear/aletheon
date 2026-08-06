pub fn deadline_ms(base:u64, timeout:u64)->u64 { base + timeout }
#[cfg(test)] mod tests { use super::*; #[test] fn normal(){assert_eq!(deadline_ms(10,5),15);} }
