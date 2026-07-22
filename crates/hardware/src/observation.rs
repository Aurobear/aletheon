//! Observation ingest with monotonic sequencing, deduplication, and staleness.

use std::collections::HashMap;

use fabric::{DeviceId, EmbodiedObservation, MonoTime};

#[derive(Default)]
pub struct ObservationIngest {
    last_seq: HashMap<(DeviceId, String), u64>,
}

impl ObservationIngest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept(
        &mut self,
        device: &DeviceId,
        observation: EmbodiedObservation,
    ) -> Option<EmbodiedObservation> {
        let key = (device.clone(), observation.source.clone());
        if self
            .last_seq
            .get(&key)
            .is_some_and(|previous| observation.sequence <= *previous)
        {
            return None;
        }
        self.last_seq.insert(key, observation.sequence);
        Some(observation)
    }
}

pub fn is_stale(observation: &EmbodiedObservation, now: MonoTime) -> bool {
    observation
        .valid_until
        .is_some_and(|deadline| deadline.is_expired_at(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::MonoDeadline;

    fn observation(source: &str, sequence: u64) -> EmbodiedObservation {
        EmbodiedObservation {
            schema: "pose".into(),
            schema_version: 1,
            source: source.into(),
            sequence,
            source_time: MonoTime(sequence),
            received_at: MonoTime(sequence),
            valid_until: None,
            confidence: 1.0,
            frame_ref: None,
            payload: serde_json::json!({}),
            evidence: vec![],
        }
    }

    #[test]
    fn rejects_duplicate_and_out_of_order_sequences_per_device_source() {
        let mut ingest = ObservationIngest::new();
        let bot = DeviceId("bot".into());
        assert!(ingest.accept(&bot, observation("pose", 1)).is_some());
        assert!(ingest.accept(&bot, observation("pose", 2)).is_some());
        assert!(ingest.accept(&bot, observation("pose", 2)).is_none());
        assert!(ingest.accept(&bot, observation("pose", 1)).is_none());
        assert!(ingest
            .accept(&DeviceId("other".into()), observation("pose", 1))
            .is_some());
    }

    #[test]
    fn staleness_uses_valid_until() {
        let mut value = observation("pose", 1);
        value.valid_until = Some(MonoDeadline::after(MonoTime(1), 10));
        assert!(!is_stale(&value, MonoTime(10)));
        assert!(is_stale(&value, MonoTime(11)));
    }
}
