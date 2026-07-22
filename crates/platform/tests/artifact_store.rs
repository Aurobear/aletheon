//! Integration tests for the ArtifactStore public API.

use platform::artifact_store::{ArtifactStore, ArtifactStoreConfig};
use tempfile::TempDir;

fn store() -> (TempDir, ArtifactStore) {
    let dir = tempfile::tempdir().unwrap();
    let config = ArtifactStoreConfig {
        root: dir.path().to_path_buf(),
        ..Default::default()
    };
    (dir, ArtifactStore::new(config).unwrap())
}

fn store_with_quota(quota_bytes: u64) -> (TempDir, ArtifactStore) {
    let dir = tempfile::tempdir().unwrap();
    let config = ArtifactStoreConfig {
        root: dir.path().to_path_buf(),
        quota_bytes,
        ..Default::default()
    };
    (dir, ArtifactStore::new(config).unwrap())
}

#[test]
fn put_retrieve_and_verify_roundtrip() {
    let (_dir, store) = store();
    let data = b"integration test frame data";
    let hash = store.put(data, "image/jpeg").expect("put should succeed");
    assert!(store.exists(&hash));
    assert!(store.verify(&hash).expect("verify should succeed"));
    // Read back
    let mut f = store.open_read(&hash).expect("open_read should succeed");
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut f, &mut buf).expect("read should succeed");
    assert_eq!(buf, data);
}

#[test]
fn deduplication_prevents_duplicate_storage() {
    let (_dir, store) = store();
    let data = b"deduplicated content";
    let h1 = store.put(data, "image/png").unwrap();
    let h2 = store.put(data, "image/png").unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn size_quota_enforcement() {
    let (_dir, store) = store_with_quota(2048);
    // First artifact fills half
    store.put(&vec![0u8; 1024], "image/jpeg").unwrap();
    // Second artifact exceeds remaining quota
    let result = store.put(&vec![1u8; 1500], "image/jpeg");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("quota exceeded"), "expected quota error, got: {}", err);
}

#[test]
fn mime_type_allowlist_filters() {
    let (_dir, store) = store();
    // Allowed types
    assert!(store.put(b"jpeg frame", "image/jpeg").is_ok());
    assert!(store.put(b"png frame", "image/png").is_ok());
    // Disallowed types
    assert!(store.put(b"html", "text/html").is_err());
    assert!(store.put(b"json", "application/json").is_err());
    assert!(store.put(b"bin", "application/octet-stream").is_err());
}

#[test]
fn path_traversal_prevention() {
    let (_dir, store) = store();
    // OpenRead with traversal patterns should be rejected
    assert!(store.open_read("../../../etc/passwd").is_err());
    assert!(store.open_read("a/b/c").is_err());
    assert!(store.open_read("abc\\def").is_err());
}

#[test]
fn nonexistent_artifact_operations() {
    let (_dir, store) = store();
    let fake_hash = "1111111111111111111111111111111111111111111111111111111111111111";
    assert!(!store.exists(fake_hash));
    assert!(store.open_read(fake_hash).is_err());
    assert!(store.verify(fake_hash).is_err());
}

#[test]
fn empty_data_is_rejected() {
    let (_dir, store) = store();
    assert!(store.put(b"", "image/jpeg").is_err());
}
