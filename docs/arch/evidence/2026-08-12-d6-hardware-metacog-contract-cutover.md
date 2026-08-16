# D6 Hardware and Metacog contract cutover

- E-stop state/event now live beside the sole Hardware emergency-stop authority;
  Hardware production and tests no longer consume Fabric's duplicate module.
- HIL evidence/result now live beside Metacog's sole verifier; its integration
  test consumes the Metacog-owned schema directly.
- Fabric removed four public/census rows and both old modules without aliases.
