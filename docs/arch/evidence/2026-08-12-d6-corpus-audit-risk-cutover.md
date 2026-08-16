# D6 Corpus audit and tool-risk cutover

The durable tool audit adapter and tool-name risk classifier are Corpus-owned
implementation details. They now live under `corpus::security`; Fabric no longer
contains filesystem/channel audit behavior or Corpus tool-name policy.

`AuditEventId`, time, and permission contracts remain Fabric inputs. No generic
risk engine was copied into Kernel.
