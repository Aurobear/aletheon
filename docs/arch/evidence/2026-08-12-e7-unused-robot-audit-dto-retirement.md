# E7 unused robot-audit DTO retirement

- `fabric::types::robot_audit::RobotAuditRecord` had no production or test
  caller. The live robot audit chain owns its record representation inside the
  Executive application adapter and did not consume this Fabric DTO.
- E7 deleted the unused module and the corresponding public inventory and
  boundary-census row. No compatibility alias or replacement schema was added.
