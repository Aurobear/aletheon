# D6 Gateway exec projection cutover

The non-interactive JSONL `ExecEvent` projection is a client wire surface. It
now belongs to `gateway_protocol::exec`; Aletheon and the temporary Executive
launcher consume that protocol owner. Fabric no longer exposes the projection.
