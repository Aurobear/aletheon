//! Closed-world adapter identifiers used only during composition.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrationKind {
    Channel,
    CodingRuntime,
    InformationSource,
    SupplementalMemory,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
            })
        {
            return Err("adapter ID must be a bounded lowercase token".into());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn integration_kind(id: &AdapterId) -> Result<IntegrationKind, String> {
    match id.as_str() {
        "telegram" => Ok(IntegrationKind::Channel),
        "pi" | "pi-rpc" => Ok(IntegrationKind::CodingRuntime),
        "google" => Ok(IntegrationKind::InformationSource),
        "gbrain" => Ok(IntegrationKind::SupplementalMemory),
        value => Err(format!("unknown adapter ID: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_static_and_unknown_ids_fail_closed() {
        assert_eq!(
            integration_kind(&AdapterId::parse("pi-rpc").unwrap()).unwrap(),
            IntegrationKind::CodingRuntime
        );
        assert!(integration_kind(&AdapterId::parse("unreviewed").unwrap()).is_err());
        assert!(AdapterId::parse("Bad Adapter").is_err());
    }
}
