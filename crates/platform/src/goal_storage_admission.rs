//! Platform adapter for Goal attempt storage admission.

use application::goal::{GoalId, GoalStorageAdmissionError, GoalStorageAdmissionPort};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::storage_quota::{StorageClass, StorageQuota, StorageReservation};

pub struct QuotaGoalStorageAdmission {
    quota: StorageQuota,
    expected_attempt_bytes: u64,
    reservations: Mutex<HashMap<GoalId, StorageReservation>>,
}

impl QuotaGoalStorageAdmission {
    pub fn new(quota: StorageQuota, expected_attempt_bytes: u64) -> Self {
        Self {
            quota,
            expected_attempt_bytes,
            reservations: Mutex::new(HashMap::new()),
        }
    }
}

impl GoalStorageAdmissionPort for QuotaGoalStorageAdmission {
    fn admit(&self, goal_id: GoalId) -> Result<(), GoalStorageAdmissionError> {
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let std::collections::hash_map::Entry::Vacant(entry) = reservations.entry(goal_id) {
            let reservation = self
                .quota
                .reserve(StorageClass::Total, self.expected_attempt_bytes, 1)
                .map_err(|error| GoalStorageAdmissionError::new(error.to_string()))?;
            entry.insert(reservation);
        }
        Ok(())
    }

    fn release(&self, goal_id: GoalId) {
        self.reservations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&goal_id);
    }
}
