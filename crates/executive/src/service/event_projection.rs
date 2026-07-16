//! Deterministic reducers and transactional projection checkpoints.

use std::{path::Path, sync::Mutex};

use fabric::{SchemaId, SpineEvent};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionDescriptor {
    pub name: &'static str,
    pub version: u32,
    pub accepted_schemas: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionCheckpoint {
    pub projection: String,
    pub version: u32,
    pub through_sequence: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionPoison {
    pub projection: String,
    pub event_id: String,
    pub sequence: u64,
    pub error: String,
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("projection descriptor is invalid: {0}")]
    InvalidDescriptor(String),
    #[error("projection {projection} requires rebuild from version {stored} to {requested}")]
    VersionMismatch {
        projection: String,
        stored: u32,
        requested: u32,
    },
    #[error("projection event sequence is not strictly ordered: {previous} then {current}")]
    NonMonotonic { previous: u64, current: u64 },
    #[error("projection {projection} poisoned at event {event_id}: {message}")]
    Poisoned {
        projection: String,
        event_id: String,
        message: String,
    },
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

pub trait EventProjection {
    type State: Default + Clone + Serialize + DeserializeOwned;

    fn descriptor(&self) -> ProjectionDescriptor;
    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError>;
}

pub struct SqliteProjectionStore {
    connection: Mutex<Connection>,
}

impl SqliteProjectionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectionError> {
        let connection = Connection::open(path)
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS event_projection_state(
                   projection TEXT PRIMARY KEY,
                   version INTEGER NOT NULL,
                   through_sequence INTEGER NOT NULL,
                   checksum TEXT NOT NULL,
                   state_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS event_projection_poison(
                   projection TEXT PRIMARY KEY,
                   event_id TEXT NOT NULL,
                   sequence INTEGER NOT NULL,
                   error TEXT NOT NULL
                 );",
            )
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn advance<P: EventProjection>(
        &self,
        projection: &P,
        events: &[SpineEvent],
    ) -> Result<(P::State, ProjectionCheckpoint), ProjectionError> {
        let descriptor = validate_descriptor(projection.descriptor())?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        let stored: Option<(u32, u64, String, String)> = transaction
            .query_row(
                "SELECT version,through_sequence,checksum,state_json
                 FROM event_projection_state WHERE projection=?1",
                params![descriptor.name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        let (mut state, mut watermark) = match stored {
            Some((version, through, _, json)) if version == descriptor.version => (
                serde_json::from_str(&json)
                    .map_err(anyhow::Error::from)
                    .map_err(ProjectionError::Storage)?,
                through,
            ),
            Some((stored, _, _, _)) => {
                return Err(ProjectionError::VersionMismatch {
                    projection: descriptor.name.into(),
                    stored,
                    requested: descriptor.version,
                })
            }
            None => (P::State::default(), 0),
        };

        let mut previous = 0;
        for event in events {
            let sequence = event.position.sequence.0;
            if previous != 0 && sequence <= previous {
                return Err(ProjectionError::NonMonotonic {
                    previous,
                    current: sequence,
                });
            }
            previous = sequence;
            if sequence <= watermark {
                continue;
            }
            if descriptor
                .accepted_schemas
                .contains(&event.schema.0.as_str())
            {
                if let Err(error) = projection.apply(&mut state, event) {
                    let poison = ProjectionPoison {
                        projection: descriptor.name.into(),
                        event_id: event.position.event_id.to_string(),
                        sequence,
                        error: error.to_string(),
                    };
                    transaction
                        .execute(
                            "INSERT INTO event_projection_poison(projection,event_id,sequence,error)
                             VALUES(?1,?2,?3,?4)
                             ON CONFLICT(projection) DO UPDATE SET
                               event_id=excluded.event_id,sequence=excluded.sequence,error=excluded.error",
                            params![poison.projection, poison.event_id, poison.sequence, poison.error],
                        )
                        .map_err(anyhow::Error::from)
                        .map_err(ProjectionError::Storage)?;
                    transaction
                        .commit()
                        .map_err(anyhow::Error::from)
                        .map_err(ProjectionError::Storage)?;
                    return Err(ProjectionError::Poisoned {
                        projection: descriptor.name.into(),
                        event_id: event.position.event_id.to_string(),
                        message: error.to_string(),
                    });
                }
            }
            watermark = sequence;
        }

        let state_json = serde_json::to_string(&state)
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        let checkpoint = ProjectionCheckpoint {
            projection: descriptor.name.into(),
            version: descriptor.version,
            through_sequence: watermark,
            checksum: checksum(state_json.as_bytes()),
        };
        transaction
            .execute(
                "INSERT INTO event_projection_state(projection,version,through_sequence,checksum,state_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(projection) DO UPDATE SET
                   version=excluded.version,through_sequence=excluded.through_sequence,
                   checksum=excluded.checksum,state_json=excluded.state_json",
                params![
                    checkpoint.projection,
                    checkpoint.version,
                    checkpoint.through_sequence,
                    checkpoint.checksum,
                    state_json
                ],
            )
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        transaction
            .execute(
                "DELETE FROM event_projection_poison WHERE projection=?1",
                params![descriptor.name],
            )
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        transaction
            .commit()
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        Ok((state, checkpoint))
    }

    pub fn rebuild<P: EventProjection>(
        &self,
        projection: &P,
        events: &[SpineEvent],
    ) -> Result<(P::State, ProjectionCheckpoint), ProjectionError> {
        let descriptor = validate_descriptor(projection.descriptor())?;
        {
            let connection = self.connection.lock().unwrap();
            connection
                .execute(
                    "DELETE FROM event_projection_state WHERE projection=?1",
                    params![descriptor.name],
                )
                .map_err(anyhow::Error::from)
                .map_err(ProjectionError::Storage)?;
        }
        self.advance(projection, events)
    }

    pub fn poison(&self, projection: &str) -> Result<Option<ProjectionPoison>, ProjectionError> {
        self.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT projection,event_id,sequence,error FROM event_projection_poison
                 WHERE projection=?1",
                params![projection],
                |row| {
                    Ok(ProjectionPoison {
                        projection: row.get(0)?,
                        event_id: row.get(1)?,
                        sequence: row.get(2)?,
                        error: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)
    }
}

fn validate_descriptor(
    descriptor: ProjectionDescriptor,
) -> Result<ProjectionDescriptor, ProjectionError> {
    if descriptor.name.trim().is_empty()
        || descriptor.version == 0
        || descriptor.accepted_schemas.is_empty()
    {
        return Err(ProjectionError::InvalidDescriptor(descriptor.name.into()));
    }
    for schema in descriptor.accepted_schemas {
        SchemaId((*schema).into())
            .validate_known()
            .map_err(|error| ProjectionError::InvalidDescriptor(error.to_string()))?;
    }
    Ok(descriptor)
}

fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
