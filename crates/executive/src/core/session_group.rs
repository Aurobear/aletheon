//! Session group — session metadata, prefix cache, memory queue, and filesystem paths.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fabric::MonoTime;
use tokio::sync::Mutex;

pub(crate) struct SessionGroup {
    pub(crate) default_session_id: Arc<tokio::sync::Mutex<String>>,
    pub(crate) session_created_at: Arc<Mutex<HashMap<String, MonoTime>>>,
    pub(crate) memory_queue: Arc<Mutex<Vec<String>>>,
    pub(crate) context_window: usize,
    pub(crate) data_dir: PathBuf,
}
