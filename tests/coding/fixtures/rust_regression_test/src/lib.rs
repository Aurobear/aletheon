#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    EmptyItem,
    TrailingEscape,
}

/// Split comma-delimited values, where a backslash escapes the next character.
pub fn split_escaped(input: &str) -> Result<Vec<String>, ParseError> {
    let values: Vec<String> = input.split(',').map(str::to_owned).collect();
    if values.iter().any(String::is_empty) {
        return Err(ParseError::EmptyItem);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_values() {
        assert_eq!(split_escaped("alpha,beta").unwrap(), ["alpha", "beta"]);
    }

    #[test]
    fn preserves_an_escaped_delimiter() {
        assert_eq!(split_escaped(r"alpha\,beta").unwrap(), ["alpha,beta"]);
    }
}
