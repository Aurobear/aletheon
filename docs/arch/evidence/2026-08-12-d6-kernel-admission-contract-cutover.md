# D6 Kernel admission contract cutover

- `AdmissionController` and `LeaseManager` are implemented and composed by
  Kernel; their canonical definitions now live in `kernel::admission::ports`.
- Executive application/bootstrap, Aletheon composition, and tests import the
  Kernel-owned ports directly.
- `BudgetController` remains temporarily in the neutral Fabric contract root:
  Runtime's Agent admission authority consumes it and the enforced dependency
  graph forbids Runtime depending on Kernel. It is therefore an AK2-25
  mechanical contracts-rename row, not a Kernel-owned compatibility alias.
