# D6 owner-specific error retirement

- The Fabric mega error taxonomy had one live caller: Corpus ToolRegistry used
  three convenience constructors. Corpus now owns a narrow `RegistryError`.
- Unused LLM/tool/sandbox/memory/IPC retry and degradation taxonomy was deleted
  rather than moved into another shared root.
- Fabric removed fifteen public/census rows and all root exports.
