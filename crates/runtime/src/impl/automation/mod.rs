//! P3 Automation / Routines System.
//!
//! Provides cron-triggered, webhook-triggered, and API-triggered automations
//! with multi-channel delivery, script pre-processing, and daily-run limits.

pub mod cron;
pub mod delivery;
pub mod script;
pub mod webhook;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info, warn};

use cron::CronParser;
use delivery::DeliveryManager;
use webhook::WebhookEvent;

// -- Public types -------------------------------------------------------------

/// Top-level scheduler that owns all configured automations and drives their
/// execution when triggers fire.
pub struct AutomationScheduler {
    automations: Vec<Automation>,
    running: bool,
    delivery: DeliveryManager,
}

/// A single automation definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub trigger: AutomationTrigger,
    pub prompt: String,
    pub script: Option<PathBuf>,
    pub skills: Vec<String>,
    pub delivery: Vec<DeliveryTarget>,
    pub model: Option<String>,
    pub daily_limit: u32,
    pub daily_count: u32,
    pub last_run: Option<u64>,
}

/// What initiates an automation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutomationTrigger {
    Cron {
        expression: String,
    },
    Webhook {
        events: Vec<String>,
        hmac_secret: String,
    },
    Api {
        endpoint: String,
    },
}

/// Where to send the agent's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeliveryTarget {
    Telegram { chat_id: Option<String> },
    Discord { channel_id: Option<String> },
    Slack { channel: Option<String> },
    Email { address: String },
    Webhook { url: String },
    Local { path: PathBuf },
    Stdout,
}

/// Result of a single automation execution.
#[derive(Debug)]
pub struct AutomationResult {
    pub automation_id: String,
    pub output: String,
    pub delivered: bool,
}

// -- Silent marker ------------------------------------------------------------

/// If the agent's response equals this marker the delivery step is skipped.
pub const SILENT_MARKER: &str = "[SILENT]";

// -- Scheduler implementation -------------------------------------------------

impl AutomationScheduler {
    pub fn new() -> Self {
        Self {
            automations: Vec::new(),
            running: false,
            delivery: DeliveryManager::new(),
        }
    }

    // -- CRUD -----------------------------------------------------------------

    /// Register a new automation.  Returns `Err` if the ID already exists.
    pub fn add_automation(&mut self, automation: Automation) -> Result<()> {
        if self.automations.iter().any(|a| a.id == automation.id) {
            return Err(anyhow!("Automation '{}' already exists", automation.id));
        }
        info!(id = %automation.id, name = %automation.name, "Automation added");
        self.automations.push(automation);
        Ok(())
    }

    /// Remove an automation by ID.  Returns `true` if it existed.
    pub fn remove_automation(&mut self, id: &str) -> bool {
        let before = self.automations.len();
        self.automations.retain(|a| a.id != id);
        let removed = self.automations.len() < before;
        if removed {
            info!(%id, "Automation removed");
        }
        removed
    }

    /// Return a read-only view of all automations.
    pub fn list_automations(&self) -> &[Automation] {
        &self.automations
    }

    /// Look up an automation by ID.
    pub fn get_automation(&self, id: &str) -> Option<&Automation> {
        self.automations.iter().find(|a| a.id == id)
    }

    // -- Cron trigger ---------------------------------------------------------

    /// Evaluate all cron-triggered automations against `current_time` and
    /// return those that should fire.
    pub fn check_cron(&self, current_time: &DateTime<Utc>) -> Vec<String> {
        let mut due = Vec::new();
        for auto in &self.automations {
            if let AutomationTrigger::Cron { expression } = &auto.trigger {
                if auto.daily_count >= auto.daily_limit {
                    debug!(id = %auto.id, "Daily limit reached, skipping");
                    continue;
                }
                if let Ok(sched) = CronParser::parse(expression) {
                    if CronParser::matches(&sched, current_time) {
                        due.push(auto.id.clone());
                    }
                }
            }
        }
        due
    }

    // -- Webhook trigger ------------------------------------------------------

    /// Match an incoming webhook event against webhook-triggered automations.
    ///
    /// The caller must pass a *verified* event -- this method does **not**
    /// re-verify HMAC signatures.
    pub fn trigger_webhook(&self, event: &WebhookEvent) -> Vec<String> {
        let mut matched = Vec::new();
        for auto in &self.automations {
            if let AutomationTrigger::Webhook { events, .. } = &auto.trigger {
                if webhook::matches_event_type(&event.event_type, events)
                    && auto.daily_count < auto.daily_limit
                {
                    matched.push(auto.id.clone());
                }
            }
        }
        matched
    }

    // -- Execute --------------------------------------------------------------

    /// Run a single automation: execute its optional script, produce an
    /// output string, and deliver it unless the output is `"[SILENT]"`.
    ///
    /// In a full system this would invoke the LLM; here we return the prompt
    /// as-is (or the script output) so the module is self-contained and
    /// testable without an LLM backend.
    pub async fn execute_automation(
        &mut self,
        id: &str,
        agent_output: &str,
    ) -> Result<AutomationResult> {
        let auto = self
            .automations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow!("Automation '{}' not found", id))?;

        // Daily-limit guard (double-check).
        if auto.daily_count >= auto.daily_limit {
            return Err(anyhow!("Daily limit reached for '{}'", id));
        }

        // Optional script pre-processing.
        let script_output = if let Some(script_path) = &auto.script {
            Some(script::ScriptRunner::run(script_path, &[]).await?)
        } else {
            None
        };

        let output = script_output.unwrap_or_else(|| agent_output.to_string());

        // Bump counters.
        auto.daily_count += 1;
        auto.last_run = Some(Utc::now().timestamp() as u64);

        // [SILENT] check.
        let delivered = if output.trim() == SILENT_MARKER {
            debug!(id = %auto.id, "Response is [SILENT], skipping delivery");
            false
        } else {
            for target in &auto.delivery {
                self.delivery
                    .deliver(target, &output)
                    .await
                    .unwrap_or_else(|e| {
                        warn!(id = %auto.id, error = %e, "Delivery failed");
                    });
            }
            true
        };

        Ok(AutomationResult {
            automation_id: id.to_string(),
            output,
            delivered,
        })
    }

    /// Reset daily counters for all automations (call at midnight).
    pub fn reset_daily_counts(&mut self) {
        for auto in &mut self.automations {
            auto.daily_count = 0;
        }
    }

    /// Start the scheduler (marks it as running).
    pub fn start(&mut self) {
        self.running = true;
        info!("Automation scheduler started");
    }

    /// Stop the scheduler.
    pub fn stop(&mut self) {
        self.running = false;
        info!("Automation scheduler stopped");
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

impl Default for AutomationScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_cron_auto(id: &str, expr: &str, limit: u32) -> Automation {
        Automation {
            id: id.to_string(),
            name: format!("test-{}", id),
            trigger: AutomationTrigger::Cron {
                expression: expr.to_string(),
            },
            prompt: "summarize".into(),
            script: None,
            skills: vec![],
            delivery: vec![DeliveryTarget::Stdout],
            model: None,
            daily_limit: limit,
            daily_count: 0,
            last_run: None,
        }
    }

    fn make_webhook_auto(id: &str, events: Vec<&str>, secret: &str) -> Automation {
        Automation {
            id: id.to_string(),
            name: format!("wh-{}", id),
            trigger: AutomationTrigger::Webhook {
                events: events.into_iter().map(String::from).collect(),
                hmac_secret: secret.to_string(),
            },
            prompt: "process event".into(),
            script: None,
            skills: vec![],
            delivery: vec![DeliveryTarget::Stdout],
            model: None,
            daily_limit: 100,
            daily_count: 0,
            last_run: None,
        }
    }

    // -- CRUD tests -----------------------------------------------------------

    #[test]
    fn add_and_list_automations() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("a1", "* * * * *", 10))
            .unwrap();
        sched
            .add_automation(make_cron_auto("a2", "0 * * * *", 5))
            .unwrap();
        assert_eq!(sched.list_automations().len(), 2);
    }

    #[test]
    fn add_duplicate_id_errors() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("dup", "* * * * *", 10))
            .unwrap();
        assert!(sched
            .add_automation(make_cron_auto("dup", "0 * * * *", 5))
            .is_err());
    }

    #[test]
    fn remove_automation() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("rm", "* * * * *", 10))
            .unwrap();
        assert!(sched.remove_automation("rm"));
        assert_eq!(sched.list_automations().len(), 0);
        // Removing again returns false.
        assert!(!sched.remove_automation("rm"));
    }

    #[test]
    fn get_automation_by_id() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("find-me", "* * * * *", 10))
            .unwrap();
        assert!(sched.get_automation("find-me").is_some());
        assert!(sched.get_automation("nope").is_none());
    }

    // -- Cron matching tests --------------------------------------------------

    #[test]
    fn cron_check_matches_due_automation() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("every5", "*/5 * * * *", 100))
            .unwrap();
        // 2026-06-07 10:00 is divisible by 5.
        let time = chrono::Utc.with_ymd_and_hms(2026, 6, 7, 10, 0, 0).unwrap();
        let due = sched.check_cron(&time);
        assert_eq!(due, vec!["every5"]);
    }

    #[test]
    fn cron_check_skips_non_matching() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("at30", "30 * * * *", 100))
            .unwrap();
        let time = chrono::Utc.with_ymd_and_hms(2026, 6, 7, 10, 0, 0).unwrap();
        let due = sched.check_cron(&time);
        assert!(due.is_empty());
    }

    // -- Daily limit tests ----------------------------------------------------

    #[test]
    fn cron_check_respects_daily_limit() {
        let mut auto = make_cron_auto("limited", "* * * * *", 2);
        auto.daily_count = 2; // already at limit
        let mut sched = AutomationScheduler::new();
        sched.add_automation(auto).unwrap();
        let time = chrono::Utc.with_ymd_and_hms(2026, 6, 7, 10, 0, 0).unwrap();
        assert!(sched.check_cron(&time).is_empty());
    }

    #[test]
    fn reset_daily_counts() {
        let mut auto = make_cron_auto("reset", "* * * * *", 10);
        auto.daily_count = 5;
        let mut sched = AutomationScheduler::new();
        sched.add_automation(auto).unwrap();
        sched.reset_daily_counts();
        assert_eq!(sched.list_automations()[0].daily_count, 0);
    }

    // -- Webhook tests --------------------------------------------------------

    #[test]
    fn webhook_trigger_matches() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_webhook_auto("wh1", vec!["push", "deploy"], "secret"))
            .unwrap();
        let event = WebhookEvent {
            event_type: "push".into(),
            payload: serde_json::json!({"ref": "main"}),
            signature: None,
        };
        assert_eq!(sched.trigger_webhook(&event), vec!["wh1"]);
    }

    #[test]
    fn webhook_trigger_no_match() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_webhook_auto("wh1", vec!["push"], "secret"))
            .unwrap();
        let event = WebhookEvent {
            event_type: "delete".into(),
            payload: serde_json::json!({}),
            signature: None,
        };
        assert!(sched.trigger_webhook(&event).is_empty());
    }

    // -- Execution + [SILENT] tests -------------------------------------------

    #[tokio::test]
    async fn execute_delivers_output() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut auto = make_cron_auto("local", "* * * * *", 10);
        auto.delivery = vec![DeliveryTarget::Local {
            path: tmp.path().to_path_buf(),
        }];
        let mut sched = AutomationScheduler::new();
        sched.add_automation(auto).unwrap();

        let result = sched
            .execute_automation("local", "agent says hello")
            .await
            .unwrap();
        assert!(result.delivered);
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(written, "agent says hello");
    }

    #[tokio::test]
    async fn execute_silent_skips_delivery() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut auto = make_cron_auto("silent", "* * * * *", 10);
        auto.delivery = vec![DeliveryTarget::Local {
            path: tmp.path().to_path_buf(),
        }];
        let mut sched = AutomationScheduler::new();
        sched.add_automation(auto).unwrap();

        let result = sched
            .execute_automation("silent", "[SILENT]")
            .await
            .unwrap();
        assert!(!result.delivered);
        // File should be empty (nothing was written).
        assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "");
    }

    #[tokio::test]
    async fn execute_increments_counters() {
        let mut sched = AutomationScheduler::new();
        sched
            .add_automation(make_cron_auto("ctr", "* * * * *", 10))
            .unwrap();

        sched.execute_automation("ctr", "out1").await.unwrap();
        sched.execute_automation("ctr", "out2").await.unwrap();

        let auto = sched.get_automation("ctr").unwrap();
        assert_eq!(auto.daily_count, 2);
        assert!(auto.last_run.is_some());
    }

    #[tokio::test]
    async fn execute_respects_daily_limit() {
        let mut auto = make_cron_auto("full", "* * * * *", 1);
        auto.daily_count = 1;
        let mut sched = AutomationScheduler::new();
        sched.add_automation(auto).unwrap();

        let result = sched.execute_automation("full", "should fail").await;
        assert!(result.is_err());
    }

    // -- Scheduler lifecycle --------------------------------------------------

    #[test]
    fn start_stop_running() {
        let mut sched = AutomationScheduler::new();
        assert!(!sched.is_running());
        sched.start();
        assert!(sched.is_running());
        sched.stop();
        assert!(!sched.is_running());
    }
}
