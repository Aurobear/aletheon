use aletheon::{select_host, CommandKind, ExitStatus, HostRoute};

#[test]
fn every_cli_mode_selects_exactly_one_host_path() {
    assert_eq!(select_host(Some(CommandKind::Core), false), HostRoute::Core);
    assert_eq!(
        select_host(Some(CommandKind::Daemon), false),
        HostRoute::Daemon
    );
    assert_eq!(select_host(Some(CommandKind::Exec), false), HostRoute::Exec);
    assert_eq!(
        select_host(Some(CommandKind::Version), false),
        HostRoute::Version
    );
    assert_eq!(
        select_host(Some(CommandKind::RestoreTerminal), false),
        HostRoute::RestoreTerminal
    );
    assert_eq!(select_host(None, true), HostRoute::Message);
    assert_eq!(select_host(None, false), HostRoute::Tui);
}

#[test]
fn delegated_host_success_propagates_to_process_status() {
    assert_eq!(ExitStatus::from_success(true), ExitStatus(0));
    assert_eq!(ExitStatus::from_success(false), ExitStatus(1));
}

#[test]
fn conscious_arbitration_mode_is_strict_and_observe_first() {
    use aletheon::daemon::parse_conscious_arbitration_mode;
    use contracts::ConsciousArbitrationMode;
    assert_eq!(
        parse_conscious_arbitration_mode(None).unwrap(),
        ConsciousArbitrationMode::Observe
    );
    assert_eq!(
        parse_conscious_arbitration_mode(Some("enforce")).unwrap(),
        ConsciousArbitrationMode::Enforce
    );
    assert!(parse_conscious_arbitration_mode(Some("warn")).is_err());
    assert!(parse_conscious_arbitration_mode(Some("ENFORCE")).is_err());
}
