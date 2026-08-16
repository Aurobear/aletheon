# D6 Application workspace-trust policy cutover

The pure workspace-trust decision types and `decide` policy now live in the
Application owner crate. Aletheon presentation and Executive's temporary
orchestration adapter consume `application::workspace_trust`; Fabric retains
only principal and workspace identity contracts.
