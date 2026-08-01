#![deny(rustdoc::broken_intra_doc_links)]

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Zero,
    Invalid,
}

/// Parse a positive worker count.
///
/// Returns [`ParseFailure`] when the input is empty, invalid, or zero.
pub fn parse_worker_count(input: &str) -> Result<u16, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    input.parse().map_err(|_| ParseError::Invalid)
}
