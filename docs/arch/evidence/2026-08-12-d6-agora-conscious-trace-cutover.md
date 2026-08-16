# D6 Agora conscious-trace cutover

The conscious workspace trace schema now lives with its durable writer and
reader in Agora. Executive emits and acceptance tests consume the Agora-owned
trace; Fabric no longer owns this workspace-specific evidence aggregate.
