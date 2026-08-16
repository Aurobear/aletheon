# D6 Runtime prompt-queue contract cutover

The session-owned prompt queue model and optimistic edit/cancel rules now live
in `runtime::prompt_queue`. Aletheon presentation and Executive's temporary
queue coordinator consume the Runtime owner. Fabric no longer exposes this
runtime lifecycle model.
