use std::path::PathBuf;

use aletheon_bin::workspace::WorkspaceArgs;
use clap::{Parser, Subcommand};
use fabric::PermissionProfileId;

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
fn cli_paths_use_the_shared_workspace_resolver() {
    let args = WorkspaceArgs {
        cwd: Some(PathBuf::from("/tmp")),
        add_dirs: vec![PathBuf::from("/var/tmp")],
    };
    let policy = args
        .resolve(
            std::path::Path::new("/tmp"),
            &PermissionProfileId::workspace_write(),
        )
        .unwrap();
    assert_eq!(policy.cwd(), std::path::Path::new("/tmp"));
    assert_eq!(policy.writable_roots().len(), 2);
}
