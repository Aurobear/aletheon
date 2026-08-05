use fixture_state_transition::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(transition(State::Done, "ready"), None); }
