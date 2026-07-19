use runtime_api::CapabilityRuntime;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RuntimeRegistry {
    runtimes: HashMap<String, Arc<dyn CapabilityRuntime>>,
    aliases: HashMap<String, String>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self { runtimes: HashMap::new(), aliases: HashMap::new() }
    }

    pub fn register(&mut self, id: &str, runtime: Arc<dyn CapabilityRuntime>) {
        for alias in runtime.manifest().aliases.iter() {
            self.aliases.insert(alias.clone(), id.to_string());
        }
        self.runtimes.insert(id.to_string(), runtime);
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn CapabilityRuntime>> {
        self.runtimes.get(id).or_else(|| {
            self.aliases.get(id).and_then(|real| self.runtimes.get(real))
        })
    }

    pub fn list_ids(&self) -> Vec<&String> {
        self.runtimes.keys().collect()
    }
}
