//! Session group — session metadata, prefix cache, memory queue, and filesystem paths.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fabric::MonoTime;
use tokio::sync::Mutex;

pub struct SessionGroup {
    pub default_session_id: Arc<tokio::sync::Mutex<String>>,
    pub session_created_at: Arc<Mutex<HashMap<String, MonoTime>>>,
    pub cached_prefix: Arc<Mutex<String>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub context_window: usize,
    pub config_prompt: String,
    pub data_dir: PathBuf,
}
