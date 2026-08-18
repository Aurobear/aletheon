#[test]
fn official_runtime_roots_create_one_kernel_clock_each() {
    let machine = include_str!("../../aletheon/src/host/core.rs");
    let user = include_str!("../../aletheon/src/host/user_runtime.rs");
    let handler = include_str!("../../aletheon/src/composition/daemon_bootstrap/request.rs");

    assert_eq!(
        machine.matches("SystemClock::new()").count(),
        0,
        "machine inference composition must not create an unused user-runtime clock"
    );
    assert_eq!(
        user.matches("SystemClock::new()").count(),
        1,
        "user composition must create one shared kernel clock"
    );
    assert_eq!(
        handler.matches("SystemClock::new()").count(),
        0,
        "request composition must receive the root clock instead of creating one"
    );
}
