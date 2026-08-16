# D6 Kernel lifecycle contract cutover

- Process/operation handles and manager traits are implemented by Kernel's
  lifecycle tables and `KernelRuntime`; they now live in `kernel::lifecycle`.
- Executive callers import the Kernel-owned operation port directly. Kernel
  tests use Kernel handles instead of Fabric compatibility paths.
- Fabric's `include/process.rs`, four public rows, and root re-exports were
  deleted without a Fabric compatibility alias.
