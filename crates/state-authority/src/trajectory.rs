use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_id: String,
    pub input: String,
    pub output: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub elapsed_ms: u64,
    pub completed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    pub is_error: bool,
    pub elapsed_ms: u64,
}

pub struct TrajectoryReader {
    turns: Vec<TurnRecord>,
}

impl TrajectoryReader {
    pub fn new() -> Self { Self { turns: vec![] } }

    pub fn append(&mut self, turn: TurnRecord) { self.turns.push(turn); }

    pub fn token_estimate(&self) -> usize {
        self.turns.iter().map(|t| t.input.len() + t.output.len()).sum::<usize>() / 4
    }

    pub fn compact(&mut self, max_tokens: usize) {
        while self.token_estimate() > max_tokens && self.turns.len() > 1 {
            self.turns.remove(0);
        }
    }
}

impl Default for TrajectoryReader {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trajectory_compacts_to_token_budget() {
        let mut tr = TrajectoryReader::new();
        for i in 0..10 {
            tr.append(TurnRecord {
                turn_id: format!("t{i}"),
                input: "x".repeat(100),
                output: "y".repeat(200),
                tool_calls: vec![],
                elapsed_ms: 100,
                completed: true,
            });
        }
        assert!(tr.turns.len() == 10);
        tr.compact(150); // ~75 tokens budget
        assert!(tr.turns.len() < 10);
    }
}
