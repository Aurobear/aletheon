//! Boot monitoring and lazy loading system.
//!
//! Manages boot phases, tracks service dependencies, and implements
//! staged lazy loading for optimal startup performance.

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabric::{wall_to_datetime, Clock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

use aletheon_kernel::chronos::Timer;

/// Boot phases representing system initialization stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BootPhase {
    /// System is initializing core components
    Initializing,
    /// Core system is ready, monitoring dependencies
    Monitoring,
    /// All critical services are operational
    Ready,
    /// System is running with reduced functionality
    Degraded,
}

impl BootPhase {
    /// Returns the priority order of the phase (lower = earlier).
    pub fn priority(&self) -> u8 {
        match self {
            BootPhase::Initializing => 0,
            BootPhase::Monitoring => 1,
            BootPhase::Ready => 2,
            BootPhase::Degraded => 3,
        }
    }

    /// Check if this phase is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, BootPhase::Ready | BootPhase::Degraded)
    }
}

/// Event recorded during boot process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootEvent {
    /// Timestamp when the event occurred
    pub timestamp: DateTime<Utc>,
    /// Boot phase when event was recorded
    pub phase: BootPhase,
    /// Component that generated the event
    pub component: String,
    /// Event description
    pub message: String,
    /// Duration in milliseconds (optional)
    pub duration_ms: Option<u64>,
}

impl BootEvent {
    /// Create a new boot event.
    pub fn new(
        timestamp: DateTime<Utc>,
        phase: BootPhase,
        component: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            timestamp,
            phase,
            component: component.into(),
            message: message.into(),
            duration_ms: None,
        }
    }

    /// Create a boot event with duration tracking.
    pub fn with_duration(
        timestamp: DateTime<Utc>,
        phase: BootPhase,
        component: impl Into<String>,
        message: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            timestamp,
            phase,
            component: component.into(),
            message: message.into(),
            duration_ms: Some(duration.as_millis() as u64),
        }
    }
}

/// Tracks dependencies between services/components.
#[derive(Debug)]
pub struct ServiceDependencyGraph {
    /// Adjacency list: service -> list of services it depends on
    dependencies: HashMap<String, HashSet<String>>,
    /// Reverse adjacency: service -> list of services that depend on it
    dependents: HashMap<String, HashSet<String>>,
}

impl ServiceDependencyGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Add a dependency: `service` depends on `dependency`.
    pub fn add_dependency(&mut self, service: impl Into<String>, dependency: impl Into<String>) {
        let service = service.into();
        let dependency = dependency.into();

        self.dependencies
            .entry(service.clone())
            .or_default()
            .insert(dependency.clone());

        self.dependents
            .entry(dependency)
            .or_default()
            .insert(service);
    }

    /// Get direct dependencies of a service.
    pub fn get_dependencies(&self, service: &str) -> Vec<String> {
        self.dependencies
            .get(service)
            .map(|deps| deps.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get services that depend on the given service.
    pub fn get_dependents(&self, service: &str) -> Vec<String> {
        self.dependents
            .get(service)
            .map(|deps| deps.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Perform topological sort to determine initialization order.
    /// Returns services in order they should be initialized.
    pub fn topological_sort(&self) -> Result<Vec<String>> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut all_services: HashSet<String> = HashSet::new();

        // Collect all services
        for (service, deps) in &self.dependencies {
            all_services.insert(service.clone());
            for dep in deps {
                all_services.insert(dep.clone());
            }
        }

        // Calculate in-degrees: count how many services depend on each service
        for (service, deps) in &self.dependencies {
            // service depends on deps, so service has incoming edges from deps
            *in_degree.entry(service.clone()).or_insert(0) += deps.len();
        }

        // Initialize services with no dependencies (in-degree 0)
        let mut queue: VecDeque<String> = VecDeque::new();
        for service in &all_services {
            if !in_degree.contains_key(service) || in_degree[service] == 0 {
                queue.push_back(service.clone());
            }
        }

        let mut result = Vec::new();
        while let Some(current) = queue.pop_front() {
            result.push(current.clone());

            // For each service that depends on current, reduce its in-degree
            if let Some(dependents) = self.dependents.get(&current) {
                for dependent in dependents {
                    if let Some(degree) = in_degree.get_mut(dependent) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent.clone());
                        }
                    }
                }
            }
        }

        if result.len() != all_services.len() {
            anyhow::bail!("Circular dependency detected in service graph");
        }

        Ok(result)
    }

    /// Check if adding a dependency would create a cycle.
    pub fn would_create_cycle(&self, service: &str, dependency: &str) -> bool {
        // BFS from dependency to see if we can reach service
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(dependency.to_string());
        visited.insert(dependency.to_string());

        while let Some(current) = queue.pop_front() {
            if current == service {
                return true;
            }

            if let Some(deps) = self.dependencies.get(&current) {
                for dep in deps {
                    if visited.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        false
    }

    /// Get all services in the graph.
    pub fn all_services(&self) -> HashSet<String> {
        let mut services = HashSet::new();
        for service in self.dependencies.keys() {
            services.insert(service.clone());
        }
        for deps in self.dependencies.values() {
            for dep in deps {
                services.insert(dep.clone());
            }
        }
        services
    }
}

impl Default for ServiceDependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Lazy loading stage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LazyLoadStage {
    /// Stage number (1-5)
    pub stage: u8,
    /// Delay before loading this stage
    pub delay: Duration,
    /// Components to load in this stage
    pub components: Vec<String>,
    /// Whether this stage is on-demand only
    pub on_demand: bool,
}

impl LazyLoadStage {
    /// Stage 1: Immediate loading (config, logging, IPC server)
    pub fn stage1_immediate() -> Self {
        Self {
            stage: 1,
            delay: Duration::ZERO,
            components: vec![
                "config".to_string(),
                "logging".to_string(),
                "ipc_server".to_string(),
            ],
            on_demand: false,
        }
    }

    /// Stage 2: +500ms (session restore, AgentRegistry)
    pub fn stage2_early() -> Self {
        Self {
            stage: 2,
            delay: Duration::from_millis(500),
            components: vec!["session_restore".to_string(), "agent_registry".to_string()],
            on_demand: false,
        }
    }

    /// Stage 3: +2s (LLM Provider, tool system)
    pub fn stage3_core() -> Self {
        Self {
            stage: 3,
            delay: Duration::from_secs(2),
            components: vec!["llm_provider".to_string(), "tool_system".to_string()],
            on_demand: false,
        }
    }

    /// Stage 4: +5s (perception sources)
    pub fn stage4_perception() -> Self {
        Self {
            stage: 4,
            delay: Duration::from_secs(5),
            components: vec!["perception_sources".to_string()],
            on_demand: false,
        }
    }

    /// Stage 5: On-demand (eBPF, FUSE)
    pub fn stage5_on_demand() -> Self {
        Self {
            stage: 5,
            delay: Duration::ZERO,
            components: vec!["ebpf".to_string(), "fuse".to_string()],
            on_demand: true,
        }
    }

    /// Get all default stages.
    pub fn default_stages() -> Vec<Self> {
        vec![
            Self::stage1_immediate(),
            Self::stage2_early(),
            Self::stage3_core(),
            Self::stage4_perception(),
            Self::stage5_on_demand(),
        ]
    }
}

/// Diagnostic information about boot process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootDiagnosis {
    /// Current boot phase
    pub current_phase: BootPhase,
    /// System resource status
    pub resource_status: ResourceStatus,
    /// Dependency service statuses
    pub dependency_statuses: HashMap<String, ServiceHealth>,
    /// Historical boot correlation (if available)
    pub historical_correlation: Option<HistoricalBootData>,
    /// List of issues detected
    pub issues: Vec<String>,
}

/// System resource status during boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatus {
    /// Available memory in MB
    pub available_memory_mb: u64,
    /// CPU usage percentage (0-100)
    pub cpu_usage_percent: f64,
    /// Disk I/O utilization percentage
    pub disk_io_percent: f64,
    /// Number of running processes
    pub process_count: u32,
}

/// Health status of a dependency service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceHealth {
    /// Service is healthy and responding
    Healthy,
    /// Service is degraded but functional
    Degraded(String),
    /// Service is unavailable
    Unavailable(String),
    /// Status unknown
    Unknown,
}

/// Historical boot data for correlation analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalBootData {
    /// Average boot time in milliseconds
    pub average_boot_time_ms: u64,
    /// Previous boot time in milliseconds
    pub previous_boot_time_ms: u64,
    /// Common failure points
    pub common_failures: Vec<String>,
    /// Boot success rate (0.0-1.0)
    pub success_rate: f64,
}

/// Main boot monitor that coordinates initialization and tracks progress.
pub struct BootMonitor {
    /// Current boot phase
    current_phase: RwLock<BootPhase>,
    /// Timeline of boot events
    timeline: RwLock<Vec<BootEvent>>,
    /// Service dependency graph
    dependency_graph: RwLock<ServiceDependencyGraph>,
    /// Lazy loading stages configuration
    lazy_stages: Vec<LazyLoadStage>,
    /// Loaded components tracking
    loaded_components: RwLock<HashSet<String>>,
    /// Boot start time (monotonic)
    start_time: fabric::MonoTime,
    /// Clock for all time operations
    clock: Arc<dyn Clock>,
}

impl BootMonitor {
    /// Create a new boot monitor with default lazy loading stages.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let start_time = clock.mono_now();
        Self {
            current_phase: RwLock::new(BootPhase::Initializing),
            timeline: RwLock::new(Vec::new()),
            dependency_graph: RwLock::new(ServiceDependencyGraph::new()),
            lazy_stages: LazyLoadStage::default_stages(),
            loaded_components: RwLock::new(HashSet::new()),
            start_time,
            clock,
        }
    }

    /// Create a boot monitor with custom lazy loading stages.
    pub fn with_stages(clock: Arc<dyn Clock>, stages: Vec<LazyLoadStage>) -> Self {
        let start_time = clock.mono_now();
        Self {
            current_phase: RwLock::new(BootPhase::Initializing),
            timeline: RwLock::new(Vec::new()),
            dependency_graph: RwLock::new(ServiceDependencyGraph::new()),
            lazy_stages: stages,
            loaded_components: RwLock::new(HashSet::new()),
            start_time,
            clock,
        }
    }

    /// Get the current boot phase.
    pub async fn current_phase(&self) -> BootPhase {
        *self.current_phase.read().await
    }

    /// Transition to a new boot phase.
    pub async fn transition_to(&self, new_phase: BootPhase) -> Result<()> {
        let mut current = self.current_phase.write().await;

        // Validate phase transition
        if new_phase.priority() < current.priority() {
            anyhow::bail!(
                "Invalid phase transition: {:?} -> {:?} (cannot go backwards)",
                current,
                new_phase
            );
        }

        let old_phase = *current;
        *current = new_phase;
        drop(current);

        info!("Boot phase transition: {:?} -> {:?}", old_phase, new_phase);

        // Record the transition event
        self.record_event(BootEvent::new(
            wall_to_datetime(self.clock.wall_now()),
            new_phase,
            "boot_monitor",
            format!("Phase transition from {:?} to {:?}", old_phase, new_phase),
        ))
        .await;

        Ok(())
    }

    /// Record a boot event.
    pub async fn record_event(&self, event: BootEvent) {
        let mut timeline = self.timeline.write().await;
        debug!(
            "Boot event: [{}] {}: {}",
            event.phase as u8, event.component, event.message
        );
        timeline.push(event);
    }

    /// Get the boot event timeline.
    pub async fn timeline(&self) -> Vec<BootEvent> {
        self.timeline.read().await.clone()
    }

    /// Add a service dependency.
    pub async fn add_dependency(&self, service: impl Into<String>, dependency: impl Into<String>) {
        let mut graph = self.dependency_graph.write().await;
        graph.add_dependency(service, dependency);
    }

    /// Get the dependency graph.
    pub async fn dependency_graph(&self) -> ServiceDependencyGraph {
        // Return a clone - in production, you'd want a read guard
        let graph = self.dependency_graph.read().await;
        ServiceDependencyGraph {
            dependencies: graph.dependencies.clone(),
            dependents: graph.dependents.clone(),
        }
    }

    /// Get topological order of services.
    pub async fn service_initialization_order(&self) -> Result<Vec<String>> {
        let graph = self.dependency_graph.read().await;
        graph.topological_sort()
    }

    /// Start lazy loading for a specific stage.
    pub async fn start_lazy_stage(&self, stage_num: u8) -> Result<()> {
        let stage = self
            .lazy_stages
            .iter()
            .find(|s| s.stage == stage_num)
            .ok_or_else(|| anyhow::anyhow!("Stage {} not found", stage_num))?;

        if stage.on_demand {
            debug!(
                "Stage {} is on-demand, skipping automatic loading",
                stage_num
            );
            return Ok(());
        }

        info!(
            "Starting lazy loading stage {}: {:?}",
            stage_num, stage.components
        );

        // Record stage start
        self.record_event(BootEvent::new(
            wall_to_datetime(self.clock.wall_now()),
            self.current_phase().await,
            "lazy_loader",
            format!(
                "Starting stage {} with components: {:?}",
                stage_num, stage.components
            ),
        ))
        .await;

        // Simulate loading delay (in real implementation, this would be actual loading)
        if !stage.delay.is_zero() {
            Timer::sleep(&*self.clock, stage.delay).await;
        }

        // Mark components as loaded
        let mut loaded = self.loaded_components.write().await;
        for component in &stage.components {
            loaded.insert(component.clone());
        }

        // Record stage completion
        self.record_event(BootEvent::with_duration(
            wall_to_datetime(self.clock.wall_now()),
            self.current_phase().await,
            "lazy_loader",
            format!("Completed stage {}: {:?}", stage_num, stage.components),
            stage.delay,
        ))
        .await;

        Ok(())
    }

    /// Load an on-demand component.
    pub async fn load_on_demand(&self, component: &str) -> Result<()> {
        // Find which stage this component belongs to
        let stage = self
            .lazy_stages
            .iter()
            .find(|s| s.components.contains(&component.to_string()));

        match stage {
            Some(s) if s.on_demand => {
                info!("Loading on-demand component: {}", component);

                self.record_event(BootEvent::new(
                    wall_to_datetime(self.clock.wall_now()),
                    self.current_phase().await,
                    "lazy_loader",
                    format!("Loading on-demand component: {}", component),
                ))
                .await;

                let mut loaded = self.loaded_components.write().await;
                loaded.insert(component.to_string());

                self.record_event(BootEvent::new(
                    wall_to_datetime(self.clock.wall_now()),
                    self.current_phase().await,
                    "lazy_loader",
                    format!("Loaded on-demand component: {}", component),
                ))
                .await;

                Ok(())
            }
            Some(_) => {
                anyhow::bail!("Component {} is not on-demand", component)
            }
            None => {
                anyhow::bail!(
                    "Component {} not found in any lazy loading stage",
                    component
                )
            }
        }
    }

    /// Check if a component is loaded.
    pub async fn is_component_loaded(&self, component: &str) -> bool {
        self.loaded_components.read().await.contains(component)
    }

    /// Get list of loaded components.
    pub async fn loaded_components(&self) -> Vec<String> {
        self.loaded_components
            .read()
            .await
            .iter()
            .cloned()
            .collect()
    }

    /// Run boot diagnosis.
    pub async fn diagnose(&self) -> BootDiagnosis {
        let current_phase = self.current_phase().await;
        let timeline = self.timeline.read().await;

        // Analyze timeline for issues
        let mut issues = Vec::new();

        // Check for slow stages
        for event in timeline.iter() {
            if let Some(duration) = event.duration_ms {
                if duration > 1000 {
                    issues.push(format!(
                        "Slow component {}: {}ms",
                        event.component, duration
                    ));
                }
            }
        }

        // Check dependency graph for cycles
        let graph = self.dependency_graph.read().await;
        if graph.topological_sort().is_err() {
            issues.push("Circular dependency detected in service graph".to_string());
        }

        BootDiagnosis {
            current_phase,
            resource_status: ResourceStatus {
                available_memory_mb: 0, // Would be populated from system
                cpu_usage_percent: 0.0,
                disk_io_percent: 0.0,
                process_count: 0,
            },
            dependency_statuses: HashMap::new(),
            historical_correlation: None,
            issues,
        }
    }

    /// Get boot elapsed time.
    pub fn elapsed(&self) -> Duration {
        let now = self.clock.mono_now();
        Duration::from_millis(now.0.saturating_sub(self.start_time.0))
    }

    /// Check if all critical components are loaded.
    pub async fn are_critical_components_ready(&self) -> bool {
        let loaded = self.loaded_components.read().await;
        let critical = ["config", "logging", "ipc_server"];

        critical.iter().all(|c| loaded.contains(*c))
    }
}

impl Default for BootMonitor {
    fn default() -> Self {
        Self::new(Arc::new(aletheon_kernel::chronos::SystemClock::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use std::sync::Arc;

    fn test_clock() -> Arc<TestClock> {
        Arc::new(TestClock::default())
    }

    #[tokio::test]
    async fn test_boot_phase_transitions() {
        let clock = test_clock();
        let monitor = BootMonitor::new(clock.clone());

        // Initial phase should be Initializing
        assert_eq!(monitor.current_phase().await, BootPhase::Initializing);

        // Valid transition: Initializing -> Monitoring
        monitor.transition_to(BootPhase::Monitoring).await.unwrap();
        assert_eq!(monitor.current_phase().await, BootPhase::Monitoring);

        // Valid transition: Monitoring -> Ready
        monitor.transition_to(BootPhase::Ready).await.unwrap();
        assert_eq!(monitor.current_phase().await, BootPhase::Ready);

        // Invalid transition: Ready -> Initializing (backwards)
        let result = monitor.transition_to(BootPhase::Initializing).await;
        assert!(result.is_err());

        // Valid transition: Ready -> Degraded
        monitor.transition_to(BootPhase::Degraded).await.unwrap();
        assert_eq!(monitor.current_phase().await, BootPhase::Degraded);
    }

    #[tokio::test]
    async fn test_dependency_tracking() {
        let mut graph = ServiceDependencyGraph::new();

        // Add dependencies
        graph.add_dependency("app", "config");
        graph.add_dependency("app", "database");
        graph.add_dependency("database", "config");

        // Check dependencies
        let app_deps = graph.get_dependencies("app");
        assert!(app_deps.contains(&"config".to_string()));
        assert!(app_deps.contains(&"database".to_string()));

        // Check dependents
        let config_dependents = graph.get_dependents("config");
        assert!(config_dependents.contains(&"app".to_string()));
        assert!(config_dependents.contains(&"database".to_string()));

        // Topological sort should work
        let order = graph.topological_sort().unwrap();
        let config_idx = order.iter().position(|s| s == "config").unwrap();
        let db_idx = order.iter().position(|s| s == "database").unwrap();
        let app_idx = order.iter().position(|s| s == "app").unwrap();

        // config should come before both database and app
        // database should come before app
        assert!(config_idx < app_idx);
        assert!(db_idx < app_idx);
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = ServiceDependencyGraph::new();

        graph.add_dependency("a", "b");
        graph.add_dependency("b", "c");

        // Adding c -> a would create a cycle
        assert!(graph.would_create_cycle("c", "a"));

        // Adding c -> d should not create a cycle
        assert!(!graph.would_create_cycle("c", "d"));
    }

    #[tokio::test]
    async fn test_lazy_loading_stages() {
        let clock = test_clock();
        let monitor = BootMonitor::new(clock.clone());

        // Stage 1 components should not be loaded initially
        assert!(!monitor.is_component_loaded("config").await);
        assert!(!monitor.is_component_loaded("logging").await);

        // Start stage 1 (immediate, no delay)
        monitor.start_lazy_stage(1).await.unwrap();

        // Stage 1 components should now be loaded
        assert!(monitor.is_component_loaded("config").await);
        assert!(monitor.is_component_loaded("logging").await);
        assert!(monitor.is_component_loaded("ipc_server").await);

        // Stage 5 is on-demand, should not load automatically
        monitor.start_lazy_stage(5).await.unwrap();
        assert!(!monitor.is_component_loaded("ebpf").await);

        // Load on-demand component
        monitor.load_on_demand("ebpf").await.unwrap();
        assert!(monitor.is_component_loaded("ebpf").await);
    }

    #[tokio::test]
    async fn test_boot_event_timeline() {
        let clock = test_clock();
        let monitor = BootMonitor::new(clock);
        let ts = wall_to_datetime(monitor.clock.wall_now());

        // Record some events
        monitor
            .record_event(BootEvent::new(
                ts,
                BootPhase::Initializing,
                "test",
                "Starting initialization",
            ))
            .await;

        monitor
            .record_event(BootEvent::with_duration(
                ts,
                BootPhase::Initializing,
                "test",
                "Initialization complete",
                Duration::from_millis(100),
            ))
            .await;

        let timeline = monitor.timeline().await;
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].message, "Starting initialization");
        assert_eq!(timeline[1].duration_ms, Some(100));
    }

    #[tokio::test]
    async fn test_boot_diagnosis() {
        let clock = test_clock();
        let monitor = BootMonitor::new(clock.clone());

        // Add a dependency
        monitor.add_dependency("app", "config").await;

        // Run diagnosis
        let diagnosis = monitor.diagnose().await;
        assert_eq!(diagnosis.current_phase, BootPhase::Initializing);
        assert!(diagnosis.issues.is_empty());
    }
}
