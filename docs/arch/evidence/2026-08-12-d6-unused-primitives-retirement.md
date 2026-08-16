# D6 unused shared primitives retirement

- `primitives/comm.rs` had no production/test caller outside its own unit tests and lowered directly into the already-deprecated legacy Envelope family. D6 deleted Command/Query/Event/Stream/Mailbox instead of preserving an ownerless-looking rich facade.
- `Narrative`, `CommitmentStatus`, and `Commitment` had no data caller. They were deleted from `primitives/cognitive.rs` and its re-export surfaces.
- `Hypothesis` remains because it is embedded in the live Agora `WorkspaceContent` schema; its owner migration remains part of the Agora/D6 cutover.
