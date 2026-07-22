use std::sync::Arc;

use mnemosyne::{
    AddFactRequest, DefaultFactUseCases, FactServiceError, FactUseCases, ListFactsRequest,
    SearchFactsRequest,
};
use tokio::sync::Mutex;

fn service() -> (tempfile::TempDir, DefaultFactUseCases) {
    let directory = tempfile::tempdir().unwrap();
    let store = mnemosyne::FactStore::open(&directory.path().join("facts.db")).unwrap();
    (
        directory,
        DefaultFactUseCases::new(Arc::new(Mutex::new(store))),
    )
}

fn add(content: &str, scope: &str) -> AddFactRequest {
    AddFactRequest {
        content: content.into(),
        scope: scope.into(),
        subject: "aletheon".into(),
        tags: "architecture,kernel".into(),
    }
}

#[tokio::test]
async fn duplicate_add_and_scoped_list_preserve_governed_defaults() {
    let (_directory, service) = service();
    let first = service
        .add(add("Kernel owns lifecycle", "session-a"))
        .await
        .unwrap();
    let duplicate = service
        .add(add("Kernel owns lifecycle", "session-a"))
        .await
        .unwrap();
    service
        .add(add("Other session", "session-b"))
        .await
        .unwrap();

    assert_eq!(first, duplicate);
    let rows = service
        .list(ListFactsRequest {
            scope: Some("session-a".into()),
            include_archived: false,
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].content, "Kernel owns lifecycle");
    assert_eq!(rows[0].scope, "session-a");
    assert_eq!(rows[0].source, "explicit");
    assert_eq!(rows[0].tier, "semantic");
    assert_eq!(rows[0].trust_score, 0.7);
}

#[tokio::test]
async fn search_show_and_missing_are_typed() {
    let (_directory, service) = service();
    let fact_id = service
        .add(add("Opaque runtime boundary", "session-a"))
        .await
        .unwrap();

    let rows = service
        .search(SearchFactsRequest {
            query: "runtime".into(),
            scope: Some("session-a".into()),
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(service.show(fact_id).await.unwrap().fact_id, fact_id);
    assert_eq!(
        service.show(fact_id + 1000).await,
        Err(FactServiceError::NotFound)
    );
    assert_eq!(
        service.show(0).await,
        Err(FactServiceError::InvalidInput("id must be positive"))
    );
}

#[tokio::test]
async fn archive_pin_and_hard_delete_are_bounded_use_cases() {
    let (_directory, service) = service();
    let archived = service.add(add("Archive me", "session-a")).await.unwrap();
    let deleted = service.add(add("Delete me", "session-a")).await.unwrap();

    assert!(service.set_pinned(archived, true).await.unwrap());
    assert!(service.show(archived).await.unwrap().pinned);
    assert!(service.forget(archived, false).await.unwrap());
    assert_eq!(service.show(archived).await.unwrap().status, "archived");

    let active = service
        .list(ListFactsRequest {
            scope: Some("session-a".into()),
            include_archived: false,
        })
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].fact_id, deleted);

    assert!(service.forget(deleted, true).await.unwrap());
    assert_eq!(service.show(deleted).await, Err(FactServiceError::NotFound));
}

#[test]
fn memory_rpc_depends_on_fact_use_cases_not_the_store() {
    let source = include_str!("../src/host/daemon/handler/rpc/rpc_memory.rs");
    assert!(source.contains(".ports"));
    for forbidden in ["subsystems", "FactStore", "fact_store", ".lock()"] {
        assert!(
            !source.contains(forbidden),
            "memory RPC contains forbidden concrete access: {forbidden}"
        );
    }
}
