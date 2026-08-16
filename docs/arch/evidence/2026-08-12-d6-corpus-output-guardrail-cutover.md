# D6 Corpus output-guardrail cutover

- Output validators normalize Corpus ToolResult execution output and had one
  production caller, Corpus ToolRunnerWithGuard.
- The guardrail and validator types now live in Corpus security.
- Fabric removed five public/census rows and the old module without an alias.
