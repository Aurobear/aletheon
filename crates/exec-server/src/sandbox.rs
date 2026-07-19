//! Fail-closed materialization of daemon-resolved process policies.

use std::path::{Path, PathBuf};

use crate::protocol::ProcessSandboxPolicy;

#[derive(Debug)]
pub struct SandboxedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Lower a resolved policy to bubblewrap's argv contract. The daemon expands
/// bounded deny globs before transport, so receiving one here is an invalid,
/// unrepresentable request rather than permission to ignore it.
pub fn materialize(
    program: &Path,
    command_args: &[String],
    environment: &std::collections::HashMap<String, String>,
    working_dir: &Path,
    policy: &ProcessSandboxPolicy,
) -> Result<SandboxedCommand, String> {
    if policy.name.trim().is_empty() {
        return Err("sandbox policy name must not be empty".into());
    }
    if !program.is_absolute() {
        return Err("sandboxed process command must be an absolute path".into());
    }
    if !working_dir.is_absolute() {
        return Err("sandboxed process working directory must be absolute".into());
    }
    if !policy.deny_globs.is_empty() {
        return Err("sandbox policy contains unexpanded deny globs".into());
    }
    for path in policy
        .read_only_roots
        .iter()
        .chain(&policy.read_write_roots)
        .chain(&policy.deny_exact)
    {
        if !path.is_absolute() {
            return Err(format!(
                "sandbox policy contains a non-absolute path: {}",
                path.display()
            ));
        }
    }
    let bwrap = which::which("bwrap")
        .map_err(|_| "bubblewrap is unavailable; sandbox policy cannot be enforced".to_string())?;
    let mut args = vec![
        "--die-with-parent".into(),
        "--unshare-pid".into(),
        "--unshare-ipc".into(),
    ];
    if policy.restrict_network {
        args.push("--unshare-net".into());
    }
    args.push("--clearenv".into());

    if policy.read_only_roots.is_empty() {
        return Err("sandbox policy has no readable roots".into());
    }
    for root in &policy.read_only_roots {
        args.extend([
            "--ro-bind".into(),
            root.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        ]);
    }
    for root in &policy.read_write_roots {
        args.extend([
            "--bind".into(),
            root.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        ]);
        for protected in [".git", ".aletheon"] {
            let path = root.join(protected);
            if path.exists() {
                args.extend([
                    "--ro-bind".into(),
                    path.to_string_lossy().into_owned(),
                    path.to_string_lossy().into_owned(),
                ]);
            }
        }
    }
    for denied in &policy.deny_exact {
        match std::fs::symlink_metadata(denied) {
            Ok(metadata) if metadata.is_dir() => {
                args.extend(["--tmpfs".into(), denied.to_string_lossy().into_owned()]);
            }
            Ok(_) => args.extend([
                "--ro-bind".into(),
                "/dev/null".into(),
                denied.to_string_lossy().into_owned(),
            ]),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect denied path {}: {error}",
                    denied.display()
                ));
            }
        }
    }
    args.extend([
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--dev-bind".into(),
        "/dev/null".into(),
        "/dev/null".into(),
        "--chdir".into(),
        working_dir.to_string_lossy().into_owned(),
    ]);
    for (key, value) in environment {
        args.extend(["--setenv".into(), key.clone(), value.clone()]);
    }
    args.push("--".into());
    args.push(program.to_string_lossy().into_owned());
    args.extend(command_args.iter().cloned());
    Ok(SandboxedCommand {
        program: bwrap,
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_globs_fail_closed_before_backend_probe() {
        let policy = ProcessSandboxPolicy {
            name: "strict".into(),
            read_only_roots: vec![PathBuf::from("/")],
            read_write_roots: vec![],
            deny_exact: vec![],
            deny_globs: vec!["**/*.pem".into()],
            restrict_network: true,
        };
        assert!(materialize(
            Path::new("/bin/sh"),
            &[],
            &Default::default(),
            Path::new("/"),
            &policy
        )
        .unwrap_err()
        .contains("unexpanded deny globs"));
    }
}
