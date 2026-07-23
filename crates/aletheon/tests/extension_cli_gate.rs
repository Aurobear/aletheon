//! Gate tests for CLI commands that must fail gracefully until implemented.
//!
//! R0 repair: assert unimplemented extension subcommands return non-zero exit
//! codes and error messages, not success text output.

use std::process::Command;

/// Path to the compiled aletheon binary.
///
/// The test binary lives in `target/debug/deps/`; the aletheon binary is in
/// `target/debug/aletheon`.  Walk up from the test executable to find it.
fn aletheon_binary() -> std::path::PathBuf {
    let test_bin = std::env::current_exe().expect("cannot find current exe");
    let target_dir = test_bin
        .parent()                       // deps/
        .and_then(|p| p.parent())       // debug/ or release/
        .expect("cannot resolve target dir");
    target_dir.join("aletheon")
}

#[test]
fn unimplemented_extension_commands_return_error() {
    let binary = aletheon_binary();

    // Commands that must fail with "not implemented" (wired through ExtensionManageService
    // but the service returns bail! for unimplemented paths).
    let cases: &[&[&str]] = &[
        &["extension", "show", "test.minimal"],
        &["extension", "enable", "test.minimal"],
        &["extension", "disable", "test.minimal"],
        &["extension", "upgrade", "/nonexistent/pkg.tar.gz"],
        &["extension", "rollback", "test.minimal"],
        &["extension", "remove", "test.minimal"],
        &["extension", "purge", "test.minimal"],
        &["extension", "import-legacy"],
    ];

    for args in cases {
        let output = Command::new(&binary)
            .args(*args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run `aletheon {}`: {e}", args.join(" ")));

        assert!(
            !output.status.success(),
            "`aletheon {}` must return non-zero; got success with stdout: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not yet") || stderr.contains("not implemented"),
            "`aletheon {}` error must say 'not yet' or 'not implemented'; got: {stderr}",
            args.join(" ")
        );
    }
}

#[test]
fn implemented_inspect_rejects_nonexistent_package() {
    let binary = aletheon_binary();
    let output = Command::new(&binary)
        .args(["extension", "inspect", "/nonexistent/pkg.tar.gz"])
        .output()
        .expect("failed to run aletheon");

    assert!(
        !output.status.success(),
        "inspect of missing package must fail"
    );
}
