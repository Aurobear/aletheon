pub fn normalize(path: &str) -> String { path.replace("//", "/") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(normalize("/a///b"), "/a/b"); }
}
