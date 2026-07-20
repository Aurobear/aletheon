use std::cmp::Ordering;

use crate::{MemoryAuthority, RecallItem, TemporalState};

fn authority_rank(authority: MemoryAuthority) -> u8 {
    match authority {
        MemoryAuthority::ApprovedCore => 0,
        MemoryAuthority::VerifiedLocalSemantic => 1,
        MemoryAuthority::LocalEpisode => 2,
        MemoryAuthority::AletheonExternal => 3,
        MemoryAuthority::ExternalReference => 4,
        MemoryAuthority::RawExperience => 5,
    }
}

pub(crate) fn state_rank(state: TemporalState) -> u8 {
    match state {
        TemporalState::Current => 0,
        TemporalState::Unknown => 1,
        TemporalState::Superseded => 2,
        TemporalState::Expired => 3,
    }
}

pub(crate) fn compare(left: &RecallItem, right: &RecallItem) -> Ordering {
    authority_rank(left.authority)
        .cmp(&authority_rank(right.authority))
        .then_with(|| state_rank(left.temporal_state).cmp(&state_rank(right.temporal_state)))
        .then_with(|| {
            right
                .metadata
                .valid_from
                .unwrap_or(right.metadata.observed_time)
                .cmp(
                    &left
                        .metadata
                        .valid_from
                        .unwrap_or(left.metadata.observed_time),
                )
        })
        .then_with(|| {
            right
                .metadata
                .observed_time
                .cmp(&left.metadata.observed_time)
        })
        .then_with(|| {
            right
                .metadata
                .confidence
                .total_cmp(&left.metadata.confidence)
        })
        .then_with(|| left.metadata.record_id.cmp(&right.metadata.record_id))
}

pub(crate) fn prefer(candidate: &RecallItem, existing: &RecallItem) -> bool {
    compare(candidate, existing).is_lt()
}
