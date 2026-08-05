#[derive(Clone)] pub struct Config { pub timeout: u64 }
pub fn merge_layer(base: Config, layer: Option<u64>) -> Config { Config { timeout: layer.unwrap_or(base.timeout) } }
pub fn effective(defaults: Config, user: Option<u64>, project: Option<u64>) -> Config { merge_layer(merge_layer(defaults, user), project) }
