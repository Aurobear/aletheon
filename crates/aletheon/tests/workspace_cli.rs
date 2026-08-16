use std::path::PathBuf;

use aletheon::workspace::WorkspaceArgs;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
struct CliFixture {
    #[command(flatten)]
    workspace: WorkspaceArgs,
    #[command(subcommand)]
    command: Option<CommandFixture>,
}

#[derive(Debug, Subcommand)]
enum CommandFixture {
    Exec,
}

#[test]
fn cli_accepts_global_workspace_options() {
    let cli = CliFixture::try_parse_from([
        "aletheon",
        "-C",
        "/tmp",
        "--add-dir",
        "/var/tmp",
        "--add-dir",
        "/opt/work",
    ])
    .unwrap();
    assert_eq!(cli.workspace.cwd, Some(PathBuf::from("/tmp")));
    assert_eq!(cli.workspace.add_dirs.len(), 2);
}

#[test]
fn global_workspace_options_are_accepted_after_a_subcommand() {
    let cli =
        CliFixture::try_parse_from(["aletheon", "exec", "-C", "/tmp", "--add-dir", "/var/tmp"])
            .unwrap();
    assert_eq!(cli.workspace.cwd, Some(PathBuf::from("/tmp")));
    assert_eq!(cli.workspace.add_dirs, vec![PathBuf::from("/var/tmp")]);
}

#[test]
fn cli_paths_delegate_as_an_explicit_host_request() {
    let args = WorkspaceArgs {
        cwd: Some(PathBuf::from("/tmp")),
        add_dirs: vec![PathBuf::from("/var/tmp")],
    };
    let exec = args.exec_launch();
    let interact = args.interact_launch();
    assert_eq!(exec.cwd, Some(PathBuf::from("/tmp")));
    assert_eq!(exec.add_dirs, vec![PathBuf::from("/var/tmp")]);
    assert_eq!(interact.cwd, exec.cwd);
    assert_eq!(interact.add_dirs, exec.add_dirs);
}
