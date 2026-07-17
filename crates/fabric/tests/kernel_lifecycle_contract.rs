use fabric::{OperationState, ProcessState};

#[test]
fn process_transition_matrix_is_total_and_exact() {
    use ProcessState::*;
    let states = [Created, Ready, Running, Waiting, Stopping, Exited, Failed];
    let allowed = [
        (Created, Ready),
        (Created, Stopping),
        (Created, Failed),
        (Ready, Running),
        (Ready, Stopping),
        (Ready, Failed),
        (Running, Waiting),
        (Running, Stopping),
        (Running, Failed),
        (Waiting, Running),
        (Waiting, Stopping),
        (Waiting, Failed),
        (Stopping, Exited),
        (Stopping, Failed),
    ];
    for from in states {
        for to in states {
            assert_eq!(
                from.can_transition_to(to),
                allowed.contains(&(from, to)),
                "unexpected Process transition {from:?} -> {to:?}"
            );
        }
    }
}

#[test]
fn operation_transition_matrix_is_total_and_exact() {
    use OperationState::*;
    let states = [Submitted, Running, Cancelling, Succeeded, Failed, Cancelled];
    let allowed = [
        (Submitted, Running),
        (Submitted, Cancelling),
        (Running, Cancelling),
        (Running, Succeeded),
        (Running, Failed),
        (Cancelling, Cancelled),
    ];
    for from in states {
        for to in states {
            assert_eq!(
                from.can_transition_to(to),
                allowed.contains(&(from, to)),
                "unexpected Operation transition {from:?} -> {to:?}"
            );
        }
    }
}
