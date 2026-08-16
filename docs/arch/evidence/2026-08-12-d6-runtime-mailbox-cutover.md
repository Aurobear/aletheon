# D6 Runtime mailbox cutover evidence

- Date: 2026-08-12
- Canonical owner: `runtime::mailbox`.
- The bounded mailbox, delivery receipt, mailbox service, and in-process registry now live together under Runtime delegation.
- Kernel and Executive consumers use the Runtime owner directly; Runtime's agent mailbox bridge uses its sibling mailbox contract.
- Fabric's mailbox implementation, module declaration, unit/integration tests, and five census/public rows were removed; the public baseline changed from 742 to 737.
- Envelope and process wire DTOs remain consumed from Fabric until their separately governed ownership cuts.
