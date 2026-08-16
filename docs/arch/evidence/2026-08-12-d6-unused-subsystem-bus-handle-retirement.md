# D6 unused Subsystem bus handle retirement

`SubsystemContext::bus` was optional and every construction set it to `None`;
no subsystem read the field. The unused field and its otherwise unreachable
`BusHandle` trait are deleted rather than promoted into a Gateway port.

Evidence: all active event-bus users receive their typed bus through their
constructor, and `rg 'BusHandle|bus_handle|bus: None' crates` is empty.
