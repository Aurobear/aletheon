//! M-H Option A regression guard: after removing the cognitive MemoryRouter,
//! the daemon's canonical store (FactStore) must still recall injected facts.

use runtime::r#impl::memory::fact_store::FactStore;

#[test]
fn factstore_remains_the_canonical_recall_store() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("fact_store.db")).unwrap();

    let id = fs
        .add_fact(
            "aletheon recalls facts via FactStore",
            "general",
            "",
            "",
            0.7,
            "semantic",
            0,
        )
        .unwrap();

    let hits = fs
        .search_facts_governed("FactStore", None, false, 0.15, 4)
        .unwrap();
    assert!(
        hits.iter().any(|f| f.fact_id == id),
        "daemon recall via FactStore must still return injected facts after MemoryRouter demotion"
    );
}
