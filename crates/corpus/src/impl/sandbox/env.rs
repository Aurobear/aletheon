use crate::r#impl::sandbox::executor::SandboxPreference;

/// Auto-detected sandbox environment information.
///
/// Inspects `/proc` and filesystem markers to determine whether the
/// current host supports namespace-based sandboxing, and recommends
/// an appropriate [`SandboxPreference`].
pub struct SandboxEnvironment {
    pub is_docker: bool,
    pub is_wsl2: bool,
    pub has_user_namespace: bool,
    pub has_seccomp: bool,
    pub kernel_version: String,
}

impl SandboxEnvironment {
    /// Detect the current runtime environment.
    pub fn detect() -> Self {
        let is_docker = std::path::Path::new("/.dockerenv").exists()
            || std::fs::read_to_string("/proc/1/cgroup")
                .map(|c| c.contains("docker"))
                .unwrap_or(false);

        let is_wsl2 = std::fs::read_to_string("/proc/version")
            .map(|v| v.contains("WSL2") || v.contains("microsoft"))
            .unwrap_or(false);

        let has_user_namespace =
            std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone")
                .map(|v| v.trim() == "1")
                .unwrap_or(false);

        let has_seccomp = std::path::Path::new("/proc/sys/kernel/seccomp").exists();

        let kernel_version =
            std::fs::read_to_string("/proc/version").unwrap_or_else(|_| "unknown".to_string());

        Self {
            is_docker,
            is_wsl2,
            has_user_namespace,
            has_seccomp,
            kernel_version,
        }
    }

    /// Recommend a [`SandboxPreference`] based on detected capabilities.
    pub fn recommended_preference(&self) -> SandboxPreference {
        if self.has_user_namespace && !self.is_docker {
            SandboxPreference::Auto
        } else if self.is_docker || self.is_wsl2 {
            SandboxPreference::BestEffort
        } else {
            SandboxPreference::BestEffort
        }
    }

    /// Return a one-line summary of the detected environment.
    pub fn summary(&self) -> String {
        format!(
            "Docker={}, WSL2={}, UserNS={}, Seccomp={}, Kernel={}",
            self.is_docker,
            self.is_wsl2,
            self.has_user_namespace,
            self.has_seccomp,
            self.kernel_version.chars().take(30).collect::<String>()
        )
    }
}
