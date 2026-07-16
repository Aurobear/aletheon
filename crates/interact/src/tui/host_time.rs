//! Client-local host clock/timer adapters. They implement Fabric contracts
//! without importing Kernel runtime mechanisms.

use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fabric::{Clock, Elapsed, MonoTime, Timer, WallTime};

pub struct ClientClock {
    started: Instant,
}
impl ClientClock {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}
impl Default for ClientClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for ClientClock {
    fn wall_now(&self) -> WallTime {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        WallTime(i64::try_from(millis).unwrap_or(i64::MAX))
    }
    fn mono_now(&self) -> MonoTime {
        MonoTime(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

pub struct ClientTimer;
impl Timer for ClientTimer {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
    async fn timeout<F>(&self, duration: Duration, future: F) -> Result<F::Output, Elapsed>
    where
        F: Future + Send,
        F::Output: Send,
    {
        tokio::time::timeout(duration, future)
            .await
            .map_err(|_| Elapsed)
    }
}
