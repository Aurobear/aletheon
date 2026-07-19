use crate::domain::Label;

pub fn parse_label(input: &str) -> Result<Label, String> {
    let value = input.trim();
    if value.is_empty() { Err("label is empty".into()) } else { Ok(Label(value.into())) }
}
