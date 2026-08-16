use std::path::PathBuf;

pub fn context(thread_id: &str, working_dir: PathBuf) -> ::contracts::PrincipalContext {
    let working_dir = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir().unwrap().join(working_dir)
    };
    ::contracts::PrincipalContext::new(
        ::contracts::PrincipalId(format!("test:{thread_id}")),
        ::contracts::LocalOsPrincipal {
            uid: nix::unistd::Uid::effective().as_raw(),
            gid: nix::unistd::Gid::effective().as_raw(),
        },
        ::contracts::ConnectionId::new(),
        ::contracts::ThreadId(thread_id.to_owned()),
        ::contracts::WorkspacePolicy::from_resolved_roots(working_dir, Vec::new()).unwrap(),
        ::contracts::PermissionProfileId::workspace_write(),
        ::contracts::ApprovalPolicy::OnRequest,
    )
}
