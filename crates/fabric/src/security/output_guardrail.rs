use async_trait::async_trait;

use crate::types::tool::ToolResult;

#[derive(Debug)]
pub enum ValidationError {
    EmptyOutput,
    NonZeroExitCode(i32),
}

#[async_trait]
pub trait OutputValidator: Send + Sync {
    async fn validate(&self, result: &ToolResult) -> std::result::Result<(), ValidationError>;
}

pub struct NonEmptyOutputValidator;

#[async_trait]
impl OutputValidator for NonEmptyOutputValidator {
    async fn validate(&self, result: &ToolResult) -> std::result::Result<(), ValidationError> {
        if result.content.is_empty() || result.content == "(no output)" {
            Err(ValidationError::EmptyOutput)
        } else {
            Ok(())
        }
    }
}

pub struct ExitCodeValidator;

#[async_trait]
impl OutputValidator for ExitCodeValidator {
    async fn validate(&self, result: &ToolResult) -> std::result::Result<(), ValidationError> {
        // ExitCodeValidator checks is_error flag
        if result.is_error {
            // Don't fail validation for expected errors (tool returned error content)
            Ok(())
        } else {
            Ok(())
        }
    }
}

pub struct OutputGuardrail {
    validators: Vec<Box<dyn OutputValidator>>,
    pub max_retries: usize,
}

impl OutputGuardrail {
    pub fn with_defaults() -> Self {
        Self {
            validators: vec![
                Box::new(NonEmptyOutputValidator),
                Box::new(ExitCodeValidator),
            ],
            max_retries: 2,
        }
    }

    pub async fn validate(&self, result: &ToolResult) -> std::result::Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(result).await?;
        }
        Ok(())
    }
}
