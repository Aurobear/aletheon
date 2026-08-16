# D6 Dasein permission-port cutover

- `PermissionAuthority` is consumed exclusively by Dasein `SelfField` policy
  review and now lives in `dasein::core::permission_authority`.
- Executive's policy implementation imports that Dasein-owned port directly.
- Fabric's policy module, public row, and census row were deleted without an alias.
