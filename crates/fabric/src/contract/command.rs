//! Stable command-line values shared by presentation adapters.
//!
//! `CommandSpec` and `ClientIntent` will extend this module. Keeping the value
//! parser free of Clap prevents the neutral Fabric contract from depending on
//! a presentation framework.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::types::evaluation::TaskKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKindArg {
    Coding,
}

impl FromStr for TaskKindArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "coding" => Ok(Self::Coding),
            other => Err(format!("unsupported task kind: {other}")),
        }
    }
}

impl fmt::Display for TaskKindArg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coding => formatter.write_str("coding"),
        }
    }
}

impl From<TaskKindArg> for TaskKind {
    fn from(value: TaskKindArg) -> Self {
        match value {
            TaskKindArg::Coding => Self::Coding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_kind_argument_round_trips_without_presentation_dependencies() {
        let value = "coding".parse::<TaskKindArg>().unwrap();
        assert_eq!(value, TaskKindArg::Coding);
        assert_eq!(value.to_string(), "coding");
        assert_eq!(TaskKind::from(value), TaskKind::Coding);
    }

    #[test]
    fn task_kind_argument_rejects_unknown_values() {
        assert!("general".parse::<TaskKindArg>().is_err());
    }
}
