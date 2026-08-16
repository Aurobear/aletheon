# D6 DaseinOps owner cutover

`DaseinOps` is implemented by and semantically owned by the Dasein crate. The
trait now lives at `dasein::DaseinOps`; Executive consumes that owner facade.
Fabric retains only the data contracts currently referenced by the facade.
