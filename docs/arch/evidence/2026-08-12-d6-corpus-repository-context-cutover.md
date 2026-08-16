# D6 Corpus repository-context cutover

The repository inspection and change-transaction tools were the only callers of
the seven `fabric::repository` data types. Their definitions now live beside
those callers in `corpus::tools::tools::repository`; Fabric no longer exports a
repository-tool model.

Evidence:

- `rg 'fabric::repository|types::repository' crates` returns no matches.
- Corpus owns every production and test caller.
- The Fabric public-type inventory and boundary census remove all seven rows.
