# D6 Corpus loop-policy cutover

- The loop detector consumes normalized ToolResult and gates Corpus tool
  execution; its state, circuit breaker, metrics, and verdict now live in Corpus.
- Dasein's policy bridge imports the Corpus projection explicitly.
- Fabric removed five public/census rows and both security modules without aliases.
