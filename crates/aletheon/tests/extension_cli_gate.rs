//! Production CLI boundary tests for the extension application service.

use std::process::Command;
use tempfile::TempDir;

fn aletheon_binary() -> std::path::PathBuf {
    let test_bin = std::env::current_exe().expect("cannot find current exe");
    test_bin
        .parent()
        .and_then(|path| path.parent())
        .expect("cannot resolve target directory")
        .join("aletheon")
}

fn run(store: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(aletheon_binary())
        .env("ALETHEON_EXTENSION_STORE_ROOT", store.path())
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run `aletheon {}`: {error}", args.join(" ")))
}

#[test]
fn lifecycle_commands_use_real_service_errors() {
    let store = TempDir::new().unwrap();
    for args in [
        &["extension", "show", "test.missing"][..],
        &["extension", "enable", "test.missing"],
        &["extension", "disable", "test.missing"],
        &["extension", "rollback", "test.missing"],
        &["extension", "remove", "test.missing"],
        &["extension", "purge", "test.missing"],
    ] {
        let output = run(&store, args);
        assert!(!output.status.success(), "`{}` unexpectedly succeeded", args.join(" "));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("not implemented") && !stderr.contains("not yet implemented"),
            "`{}` still reached a placeholder: {stderr}",
            args.join(" ")
        );
    }
}

#[test]
fn empty_legacy_import_is_a_successful_noop() {
    let store = TempDir::new().unwrap();
    let output = run(&store, &["extension", "import-legacy"]);
    assert!(
        output.status.success(),
        "empty import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Imported 0"));
}

#[test]
fn inspect_rejects_nonexistent_package() {
    let store = TempDir::new().unwrap();
    let output = run(
        &store,
        &["extension", "inspect", "/nonexistent/pkg.tar.gz"],
    );
    assert!(!output.status.success());
}
