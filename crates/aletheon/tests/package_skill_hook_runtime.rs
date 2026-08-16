use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aletheon::extensions::extension_coordinator::ExtensionCoordinator;
use aletheon::extensions::extension_snapshot::{ExtensionRuntimeView, ExtensionSnapshotCompiler};
use aletheon::wiring::daemon::bootstrap::extension_publisher::DaemonExtensionRuntimePublisher;
use corpus::hook::{HookContext, HookPoint, HookResult};
use corpus::tools::tools::skill_tools::SharedSkills;
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::Mutex;

fn package(root: &Path, marker: &Path) -> PathBuf {
    let source = root.join("source");
    let skill = source.join("assets/skills/review/SKILL.md");
    let skill_script = source.join("assets/skills/review/scripts/review.sh");
    let hook = source.join("assets/hooks/audit.toml");
    let hook_script = source.join("payload/audit.sh");
    for path in [&skill, &skill_script, &hook, &hook_script] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    let manifest = r#"schema_version = 1
[package]
id = "aurb.core"
version = "1.0.0"
description = "Skill and Hook runtime fixture"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "skill"
id = "skill.review"
path = "assets/skills/review/SKILL.md"
[[assets]]
kind = "hook"
id = "hook.audit"
path = "assets/hooks/audit.toml"
"#;
    let skill_body = r#"---
name: review
description: Review a change
tools:
  - name: aurb_review
    description: Run packaged review
    script: review.sh
    permission: L0
---
# Review
Use the packaged review workflow.
"#;
    let skill_script_body = "#!/bin/sh\necho reviewed\n";
    let hook_body =
        "[hook]\nname = \"audit\"\npoint = \"PostTurn\"\nscript = \"payload/audit.sh\"\n";
    let hook_script_body = format!(
        "#!/bin/sh\nprintf 'post-turn\\n' >> '{}'\n",
        marker.display()
    );
    let files = [
        ("extension.toml", manifest.as_bytes()),
        ("assets/skills/review/SKILL.md", skill_body.as_bytes()),
        (
            "assets/skills/review/scripts/review.sh",
            skill_script_body.as_bytes(),
        ),
        ("assets/hooks/audit.toml", hook_body.as_bytes()),
        ("payload/audit.sh", hook_script_body.as_bytes()),
    ];
    for (relative, content) in files {
        fs::write(source.join(relative), content).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&skill_script, &hook_script] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let mut checksums = String::new();
    for (relative, content) in files {
        checksums.push_str(&format!("{:x}  {relative}\n", Sha256::digest(content)));
    }
    fs::write(source.join("checksums.sha256"), checksums).unwrap();

    let archive = root.join("aurb-core-1.0.0.tar.gz");
    let encoder = GzEncoder::new(fs::File::create(&archive).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all(".", source).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    archive
}

fn context() -> HookContext {
    HookContext {
        point: HookPoint::PostTurn,
        session_id: "runtime-test".into(),
        turn_count: 1,
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: None,
        metadata: std::collections::HashMap::new(),
    }
}

#[tokio::test]
async fn enabled_package_skill_and_hook_are_visible_without_restart() {
    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("hook-receipts.txt");
    let archive = package(temp.path(), &marker);
    let tools = Arc::new(Mutex::new(corpus::ToolRegistry::new()));
    let hooks = Arc::new(Mutex::new(corpus::HookRegistry::new(Arc::new(
        kernel::chronos::TestClock::default(),
    ))));
    let skills = SharedSkills::new(Arc::new(Vec::new()));
    let publisher = Arc::new(DaemonExtensionRuntimePublisher::new(
        tools.clone(),
        hooks.clone(),
        skills.clone(),
    ));
    let coordinator = ExtensionCoordinator::new(
        &temp.path().join("store"),
        ExtensionSnapshotCompiler::default(),
        publisher,
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
    .unwrap();

    coordinator
        .install("operator:test", &archive, false)
        .await
        .unwrap();
    coordinator
        .enable("operator:test", "aurb.core", true)
        .await
        .unwrap();

    assert!(skills.snapshot().iter().any(|skill| skill.name == "review"));
    assert!(tools.lock().await.get("aurb_review").is_some());
    assert!(hooks
        .lock()
        .await
        .list()
        .iter()
        .any(|hook| hook.name == "aurb.core:audit"));
    assert!(matches!(
        hooks.lock().await.execute(&context()).await,
        HookResult::Continue
    ));
    assert_eq!(fs::read_to_string(&marker).unwrap(), "post-turn\n");

    coordinator
        .disable("operator:test", "aurb.core")
        .await
        .unwrap();
    assert!(!skills.snapshot().iter().any(|skill| skill.name == "review"));
    assert!(tools.lock().await.get("aurb_review").is_none());
    assert!(!hooks
        .lock()
        .await
        .list()
        .iter()
        .any(|hook| hook.name == "aurb.core:audit"));
    hooks.lock().await.execute(&context()).await;
    assert_eq!(fs::read_to_string(&marker).unwrap(), "post-turn\n");
}
