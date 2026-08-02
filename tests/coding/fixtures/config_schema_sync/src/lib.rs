#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub retries: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { retries: 3 }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.retries == 0 {
            return Err("retries must be positive");
        }
        Ok(())
    }
}

pub fn checked_in_schema() -> &'static str {
    include_str!("../schema/config.schema.json")
}
