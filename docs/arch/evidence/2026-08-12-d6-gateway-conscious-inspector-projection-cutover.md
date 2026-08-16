# D6 Gateway conscious-inspector projection cutover

The sanitized read-only conscious-core inspector DTOs now live in
`gateway_protocol::conscious_core`. Executive produces the projection and
Interact renders it through the Gateway protocol owner. Fabric no longer owns
this presentation contract.
