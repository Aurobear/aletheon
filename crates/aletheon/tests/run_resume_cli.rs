use std::path::{Path, PathBuf};
use std::process::Command;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_aletheon"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn run_resume_and_completion_are_discoverable_from_canonical_help() {
    let root = run(&["--help"]);
    assert!(root.status.success());
    let root = String::from_utf8(root.stdout).unwrap();
    for command in ["run", "resume", "completion"] {
        assert!(root.contains(command), "root help omitted {command}");
    }

    let run_help = run(&["run", "--help"]);
    assert!(run_help.status.success());
    assert!(String::from_utf8(run_help.stdout)
        .unwrap()
        .contains("--resume <SESSION>"));

    let resume_help = run(&["resume", "--help"]);
    assert!(resume_help.status.success());
    assert!(String::from_utf8(resume_help.stdout)
        .unwrap()
        .contains("[SESSION]"));
}

#[test]
fn completion_command_prints_the_generated_assets_exactly() {
    let root = repository_root();
    for (shell, asset) in [
        ("bash", "scripts/completions/aletheon.bash"),
        ("zsh", "scripts/completions/aletheon.zsh"),
    ] {
        let output = run(&["completion", shell]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout,
            std::fs::read(root.join(asset)).unwrap(),
            "installed command output drifted from {asset}"
        );
    }
}
