//! Sandbox primitives — namespace isolation, cgroups v2, seccomp filtering.
//!
//! These are low-level building blocks. Higher-level orchestration (e.g.
//! registering as a Level 0 backend in argos-sandbox) is out of scope here.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// cBPF types for seccomp
// ---------------------------------------------------------------------------

/// A single cBPF instruction.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// Header for a cBPF program passed to prctl(PR_SET_SECCOMP).
#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

/// Compiled cBPF program for seccomp filtering.
pub struct BpfProgram {
    instructions: Vec<SockFilter>,
}

impl BpfProgram {
    /// Apply this BPF program via `prctl(PR_SET_SECCOMP)`.
    ///
    /// # Safety
    /// This enables seccomp mode 2 (SECCOMP_MODE_FILTER) for the calling
    /// thread.  The filter cannot be removed once applied.
    pub fn apply(&self) -> Result<()> {
        use libc::{prctl, PR_SET_SECCOMP, SECCOMP_MODE_FILTER};

        let prog = SockFprog {
            len: self.instructions.len() as u16,
            filter: self.instructions.as_ptr(),
        };

        let ret = unsafe {
            prctl(
                PR_SET_SECCOMP,
                SECCOMP_MODE_FILTER,
                &prog as *const _ as *const libc::c_void,
            )
        };
        if ret != 0 {
            anyhow::bail!(
                "prctl(PR_SET_SECCOMP) failed: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NamespaceSandbox — user+pid+mount namespace isolation via unshare(1)
// ---------------------------------------------------------------------------

/// Native namespace isolation (replaces shelling out to bubblewrap for simple
/// cases). Uses `unshare --user --pid --mount --fork` to create a disposable
/// execution environment.
pub struct NamespaceSandbox;

impl NamespaceSandbox {
    /// Execute `cmd` inside a new user+pid+mount namespace.
    ///
    /// Returns the child's `ExitStatus`. The child is forked by `unshare`, so
    /// the caller blocks until it exits.
    pub fn exec(cmd: &str, args: &[&str]) -> Result<std::process::ExitStatus> {
        use std::process::Command;
        let status = Command::new("unshare")
            .args(["--user", "--pid", "--mount", "--fork", "--"])
            .arg(cmd)
            .args(args)
            .status()
            .context("Failed to exec in namespace")?;
        Ok(status)
    }
}

// ---------------------------------------------------------------------------
// CgroupManager — cgroups v2 resource controller
// ---------------------------------------------------------------------------

/// Manages a single cgroup v2 subtree under `/sys/fs/cgroup/`.
///
/// Requires either root or write access to the cgroup filesystem.
pub struct CgroupManager {
    path: PathBuf,
}

impl CgroupManager {
    /// Create a new cgroup named `argos-<id>`.
    pub fn create(id: &str) -> Result<Self> {
        let path = PathBuf::from(format!("/sys/fs/cgroup/argos-{id}"));
        fs::create_dir_all(&path)
            .context("Failed to create cgroup. Need root or cgroup write access.")?;
        Ok(Self { path })
    }

    /// Set CPU bandwidth limit (`cpu.max`: `quota_us period_us`).
    pub fn set_cpu_limit(&self, quota_us: u64, period_us: u64) -> Result<()> {
        fs::write(self.path.join("cpu.max"), format!("{quota_us} {period_us}"))
            .context("Failed to set CPU limit")?;
        Ok(())
    }

    /// Set memory limit in bytes (`memory.max`).
    pub fn set_memory_limit(&self, bytes: u64) -> Result<()> {
        fs::write(self.path.join("memory.max"), bytes.to_string())
            .context("Failed to set memory limit")?;
        Ok(())
    }

    /// Set maximum number of processes (`pids.max`).
    pub fn set_pid_limit(&self, max: u32) -> Result<()> {
        fs::write(self.path.join("pids.max"), max.to_string())
            .context("Failed to set PID limit")?;
        Ok(())
    }

    /// Move a process (by PID) into this cgroup.
    pub fn add_process(&self, pid: u32) -> Result<()> {
        fs::write(self.path.join("cgroup.procs"), pid.to_string())
            .context("Failed to add process to cgroup")?;
        Ok(())
    }

    /// Remove the cgroup directory (must be empty of processes first).
    pub fn destroy(&self) -> Result<()> {
        fs::remove_dir(&self.path).context("Failed to remove cgroup")?;
        Ok(())
    }

    /// Expose the cgroup filesystem path (for testing / inspection).
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

// ---------------------------------------------------------------------------
// SeccompPolicy + SeccompCompiler — syscall allow/deny lists
// ---------------------------------------------------------------------------

/// A declarative seccomp policy: lists of syscall numbers to allow or deny.
pub struct SeccompPolicy {
    pub allow: Vec<i64>,
    pub deny: Vec<i64>,
}

impl SeccompPolicy {
    /// Default whitelist — common syscalls needed for a minimal Linux process.
    ///
    /// Denied: mount, umount2, pivot_root, ptrace, process_vm_readv, reboot,
    /// kexec_load.
    pub fn default_policy() -> Self {
        Self {
            allow: vec![
                0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12,
                13, // read, write, open, close, stat, fstat, lseek, mmap, mprotect, munmap, brk, ioctl*
                16, 17, 20, 21, 22, 32, 33, 35, 39, 41,
                42, // ioctl, writev, access, pipe, select, dup, dup2, nanosleep, getpid, socket, connect
                56, 57, 58, 59, 60, 61, 62, // clone, fork, execve, exit, wait4, kill, uname
            ],
            deny: vec![
                165, 166, 167, // mount, umount2, pivot_root
                101, 310, // ptrace, process_vm_readv
                169, 246, // reboot, kexec_load
            ],
        }
    }
}

/// A compiled seccomp rule: a single syscall number mapped to an action.
#[derive(Debug, Clone)]
pub struct SeccompRule {
    pub syscall_nr: i64,
    pub action: SeccompAction,
}

/// Action to take when a seccomp rule matches.
#[derive(Debug, Clone)]
pub enum SeccompAction {
    Allow,
    Kill,
    Log,
}

/// Compiles a [`SeccompPolicy`] into a flat list of [`SeccompRule`]s.
///
/// This is a simplified representation; real BPF emission is out of scope for
/// the driver layer and belongs in the sandbox crate.
pub struct SeccompCompiler;

impl SeccompCompiler {
    /// Compile a policy into an ordered list of rules.
    pub fn compile(policy: &SeccompPolicy) -> Vec<SeccompRule> {
        policy
            .allow
            .iter()
            .map(|&nr| SeccompRule {
                syscall_nr: nr,
                action: SeccompAction::Allow,
            })
            .chain(policy.deny.iter().map(|&nr| SeccompRule {
                syscall_nr: nr,
                action: SeccompAction::Kill,
            }))
            .collect()
    }

    /// Compile a policy into a real cBPF program suitable for seccomp.
    ///
    /// The generated program:
    /// 1. Loads the syscall number from `seccomp_data`.
    /// 2. For each denied syscall, jumps to a `SECCOMP_RET_KILL` verdict.
    /// 3. Falls through to `SECCOMP_RET_ALLOW` for everything else.
    pub fn compile_bpf(policy: &SeccompPolicy) -> BpfProgram {
        let mut instructions = Vec::new();

        // BPF_LD | BPF_W | BPF_ABS — load 32-bit word from seccomp_data at offset 0
        // (offset 0 is the syscall number on all Linux architectures).
        instructions.push(SockFilter {
            code: 0x20, // BPF_LD | BPF_W | BPF_ABS
            jt: 0,
            jf: 0,
            k: 0, // offsetof(struct seccomp_data, nr)
        });

        // For each denied syscall: if syscall_nr == nr → KILL.
        for &nr in &policy.deny {
            instructions.push(SockFilter {
                code: 0x15, // BPF_JMP | BPF_JEQ | BPF_K
                jt: 0,      // match → next instruction (KILL)
                jf: 1,      // no match → skip 1
                k: nr as u32,
            });
            instructions.push(SockFilter {
                code: 0x06, // BPF_RET
                jt: 0,
                jf: 0,
                k: 0x0000_0000, // SECCOMP_RET_KILL
            });
        }

        // Default verdict: ALLOW.
        instructions.push(SockFilter {
            code: 0x06, // BPF_RET
            jt: 0,
            jf: 0,
            k: 0x7FFF_0000, // SECCOMP_RET_ALLOW
        });

        BpfProgram { instructions }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_exec_true() {
        // `true` should succeed in a namespace
        let status = NamespaceSandbox::exec("true", &[]).unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_namespace_exec_false() {
        // `false` should exit non-zero
        let status = NamespaceSandbox::exec("false", &[]).unwrap();
        assert!(!status.success());
    }

    #[test]
    fn test_cgroup_create_and_destroy() {
        let id = format!("test-{}", std::process::id());
        // Only run the full lifecycle if we have cgroup write access
        if let Ok(cg) = CgroupManager::create(&id) {
            assert!(cg.path().exists());
            // Clean up — ignore errors (may not have permission to remove)
            let _ = cg.destroy();
        }
    }

    #[test]
    fn test_seccomp_compile_non_empty() {
        let policy = SeccompPolicy::default_policy();
        let rules = SeccompCompiler::compile(&policy);
        assert!(!rules.is_empty());
    }

    #[test]
    fn test_seccomp_compile_mount_denied() {
        let policy = SeccompPolicy::default_policy();
        let rules = SeccompCompiler::compile(&policy);

        // mount (165) should be in the deny list with Kill action
        let mount_rule = rules.iter().find(|r| r.syscall_nr == 165);
        assert!(mount_rule.is_some(), "mount syscall should be present");
        assert!(
            matches!(mount_rule.unwrap().action, SeccompAction::Kill),
            "mount should be denied with Kill"
        );
    }

    #[test]
    fn test_seccomp_compile_read_allowed() {
        let policy = SeccompPolicy::default_policy();
        let rules = SeccompCompiler::compile(&policy);

        // read (0) should be in the allow list
        let read_rule = rules.iter().find(|r| r.syscall_nr == 0);
        assert!(read_rule.is_some(), "read syscall should be present");
        assert!(
            matches!(read_rule.unwrap().action, SeccompAction::Allow),
            "read should be allowed"
        );
    }

    #[test]
    fn test_seccomp_default_policy_contains_both_allow_and_deny() {
        let policy = SeccompPolicy::default_policy();
        assert!(!policy.allow.is_empty(), "allow list should not be empty");
        assert!(!policy.deny.is_empty(), "deny list should not be empty");
        // No overlap
        for nr in &policy.deny {
            assert!(
                !policy.allow.contains(nr),
                "syscall {nr} should not appear in both allow and deny"
            );
        }
    }

    // -----------------------------------------------------------------------
    // cBPF compilation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bpf_compile_returns_instructions() {
        let policy = SeccompPolicy::default_policy();
        let prog = SeccompCompiler::compile_bpf(&policy);
        assert!(
            !prog.instructions.is_empty(),
            "BPF program should contain at least one instruction"
        );
    }

    #[test]
    fn test_bpf_first_instruction_is_load() {
        let policy = SeccompPolicy::default_policy();
        let prog = SeccompCompiler::compile_bpf(&policy);
        // First instruction must be LD [0] — opcode 0x20
        assert_eq!(
            prog.instructions[0].code, 0x20,
            "first instruction should be BPF_LD|BPF_W|BPF_ABS (0x20)"
        );
        assert_eq!(
            prog.instructions[0].k, 0,
            "first instruction should load from offset 0 (syscall number)"
        );
    }

    #[test]
    fn test_bpf_last_instruction_is_allow() {
        let policy = SeccompPolicy::default_policy();
        let prog = SeccompCompiler::compile_bpf(&policy);
        let last = prog
            .instructions
            .last()
            .expect("program should not be empty");
        assert_eq!(last.code, 0x06, "last instruction should be BPF_RET (0x06)");
        assert_eq!(
            last.k, 0x7FFF_0000,
            "last instruction should return SECCOMP_RET_ALLOW"
        );
    }
}
