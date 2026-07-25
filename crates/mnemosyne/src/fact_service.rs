//! Request-safe fact-memory use cases.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::adapters::storage::fact_store::{FactRow, FactStore};

#[derive(Debug, Clone, PartialEq)]
pub struct AddFactRequest {
    pub content: String,
    pub scope: String,
    pub subject: String,
    pub tags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListFactsRequest {
    pub scope: Option<String>,
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchFactsRequest {
    pub query: String,
    pub scope: Option<String>,
}

/// Stable request-facing projection. The database and its locks stay private.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactView {
    pub fact_id: i64,
    pub content: String,
    pub category: String,
    pub tags: String,
    pub source_path: String,
    pub trust_score: f64,
    pub retrieval_count: i64,
    pub helpful_count: i64,
    pub tier: String,
    pub ttl_days: i64,
    pub created_at: String,
    pub updated_at: String,
    pub scope: String,
    pub source: String,
    pub status: String,
    pub pinned: bool,
    pub subject: String,
}

impl From<FactRow> for FactView {
    fn from(row: FactRow) -> Self {
        Self {
            fact_id: row.fact_id,
            content: row.content,
            category: row.category,
            tags: row.tags,
            source_path: row.source_path,
            trust_score: row.trust_score,
            retrieval_count: row.retrieval_count,
            helpful_count: row.helpful_count,
            tier: row.tier,
            ttl_days: row.ttl_days,
            created_at: row.created_at,
            updated_at: row.updated_at,
            scope: row.scope,
            source: row.source,
            status: row.status,
            pinned: row.pinned,
            subject: row.subject,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FactServiceError {
    #[error("fact not found")]
    NotFound,
    #[error("invalid fact input: {0}")]
    InvalidInput(&'static str),
    #[error("fact store operation failed: {0}")]
    Store(String),
}

#[async_trait]
pub trait FactUseCases: Send + Sync {
    async fn add(&self, request: AddFactRequest) -> Result<i64, FactServiceError>;
    async fn list(&self, request: ListFactsRequest) -> Result<Vec<FactView>, FactServiceError>;
    async fn search(&self, request: SearchFactsRequest) -> Result<Vec<FactView>, FactServiceError>;
    async fn show(&self, fact_id: i64) -> Result<FactView, FactServiceError>;
    async fn forget(&self, fact_id: i64, hard: bool) -> Result<bool, FactServiceError>;
    async fn set_pinned(&self, fact_id: i64, pinned: bool) -> Result<bool, FactServiceError>;
}

pub struct DefaultFactUseCases {
    store: Arc<Mutex<FactStore>>,
}

impl DefaultFactUseCases {
    pub fn new(store: Arc<Mutex<FactStore>>) -> Self {
        Self { store }
    }

    fn valid_id(fact_id: i64) -> Result<(), FactServiceError> {
        if fact_id > 0 {
            Ok(())
        } else {
            Err(FactServiceError::InvalidInput("id must be positive"))
        }
    }

    fn store_error(error: anyhow::Error) -> FactServiceError {
        FactServiceError::Store(error.to_string())
    }
}

#[async_trait]
impl FactUseCases for DefaultFactUseCases {
    async fn add(&self, request: AddFactRequest) -> Result<i64, FactServiceError> {
        self.store
            .lock()
            .await
            .add_fact_governed(
                &request.content,
                "general",
                &request.tags,
                &request.scope,
                "explicit",
                &request.subject,
                0.7,
                "semantic",
                0,
            )
            .map_err(Self::store_error)
    }

    async fn list(&self, request: ListFactsRequest) -> Result<Vec<FactView>, FactServiceError> {
        self.store
            .lock()
            .await
            .list_facts(request.scope.as_deref(), request.include_archived, 50)
            .map(|rows| rows.into_iter().map(FactView::from).collect())
            .map_err(Self::store_error)
    }

    async fn search(&self, request: SearchFactsRequest) -> Result<Vec<FactView>, FactServiceError> {
        self.store
            .lock()
            .await
            .search_facts_governed(&request.query, request.scope.as_deref(), false, 0.15, 20)
            .map(|rows| rows.into_iter().map(FactView::from).collect())
            .map_err(Self::store_error)
    }

    async fn show(&self, fact_id: i64) -> Result<FactView, FactServiceError> {
        Self::valid_id(fact_id)?;
        self.store
            .lock()
            .await
            .get_fact(fact_id)
            .map_err(Self::store_error)?
            .map(FactView::from)
            .ok_or(FactServiceError::NotFound)
    }

    async fn forget(&self, fact_id: i64, hard: bool) -> Result<bool, FactServiceError> {
        Self::valid_id(fact_id)?;
        let store = self.store.lock().await;
        if hard {
            store.delete_fact(fact_id)
        } else {
            store.set_status(fact_id, "archived")
        }
        .map_err(Self::store_error)
    }

    async fn set_pinned(&self, fact_id: i64, pinned: bool) -> Result<bool, FactServiceError> {
        Self::valid_id(fact_id)?;
        self.store
            .lock()
            .await
            .set_pinned(fact_id, pinned)
            .map_err(Self::store_error)
    }
}
