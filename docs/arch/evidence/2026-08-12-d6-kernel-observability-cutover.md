# D6 Kernel observability cutover

Kernel debug facts (`DebugEvent`, `DebugLevel`, `Tracepoint`, `DebugSink`) and
their local observability/file adapters now live under `kernel::{debug,
debug_bus}`. Aletheon and the temporary Executive host consume the Kernel owner;
Fabric no longer contains implementation-bearing debug infrastructure.
