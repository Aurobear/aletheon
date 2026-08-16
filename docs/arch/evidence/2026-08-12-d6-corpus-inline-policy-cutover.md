# D6 Corpus inline-policy cutover

The legacy inline tool policy engine now lives in Corpus, its execution owner.
Dasein's compatibility policy bridge consumes that Corpus policy rather than a
Fabric implementation. No generic policy engine was copied into Kernel.
