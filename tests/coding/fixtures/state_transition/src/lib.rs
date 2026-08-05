#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum State { New, Ready, Done }
pub fn transition(state: State, event: &str) -> Option<State> { match (state,event) { (State::New,"ready") => Some(State::Ready), (_,"done") => Some(State::Done), _ => Some(state) } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn behavior() { assert_eq!(transition(State::Done, "ready"), None); assert_eq!(transition(State::New, "ready"), Some(State::Ready)); }
}
