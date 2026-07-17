use std::path::PathBuf;

pub fn context(thread_id: &str, working_dir: PathBuf) -> fabric::PrincipalContext {
    let working_dir = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir().unwrap().join(working_dir)
    };
    fabric::PrincipalContext::new(
        fabric::PrincipalId(format!("test:{thread_id}")),
        fabric::LocalOsPrincipal {
            uid: nix::unistd::Uid::effective().as_raw(),
            gid: nix::unistd::Gid::effective().as_raw(),
        },
        fabric::ConnectionId::new(),
        fabric::ThreadId(thread_id.to_owned()),
        fabric::WorkspacePolicy::from_resolved_roots(working_dir, Vec::new()).unwrap(),
        fabric::PermissionProfileId::workspace_write(),
        fabric::ApprovalPolicy::OnRequest,
    )
}
