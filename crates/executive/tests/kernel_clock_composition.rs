#[test]
fn official_runtime_roots_create_one_kernel_clock_each() {
    let machine = include_str!("../src/core/runtime_core.rs");
    let user = include_str!("../src/composition/user_runtime/mod.rs");
    let handler = include_str!("../src/host/daemon/bootstrap/request.rs");

    assert_eq!(
        machine.matches("SystemClock::new()").count(),
        1,
        "machine composition must create one shared kernel clock"
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
