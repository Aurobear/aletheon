//! ContextSpace integration tests — InMemorySpaceManager fork, attach, and isolation.

use aletheon_kernel::space::InMemorySpaceManager;
use fabric::{ContextBinding, ProcessId, SpaceId, SpaceManager};

/// Test 1: fork_preserves_bindings — parent has binding, fork, child has same binding.
#[tokio::test]
async fn fork_preserves_bindings() {
    let mgr = InMemorySpaceManager::new();
    let parent_id = SpaceId::new();
    let owner = ProcessId::new();

    // Attach a binding to the parent.
    let binding = ContextBinding::Session("parent-session".into());
    mgr.attach_region(parent_id, binding.clone())
        .await
        .expect("attach to parent should succeed");

    // Fork a child from the parent.
    let child_id = mgr
        .fork_space(parent_id, owner)
        .await
        .expect("fork from parent should succeed");

    // The child must be a distinct space.
    assert_ne!(parent_id, child_id, "fork must produce a new SpaceId");

    // The child must have inherited the parent's bindings.
    let child_bindings = mgr
        .get_bindings(child_id)
        .expect("child space must exist after fork");
    assert_eq!(
        child_bindings.len(),
        1,
        "child must have exactly one binding"
    );
    assert_eq!(
        child_bindings[0], binding,
        "child binding must match parent"
    );
}

/// Test 2: fork_creates_new_id — fork produces a different SpaceId.
#[tokio::test]
async fn fork_creates_new_id() {
    let mgr = InMemorySpaceManager::new();
    let parent_id = SpaceId::new();
    let owner = ProcessId::new();

    let child1 = mgr
        .fork_space(parent_id, owner)
        .await
        .expect("first fork should succeed");
    let child2 = mgr
        .fork_space(parent_id, owner)
        .await
        .expect("second fork should succeed");

    assert_ne!(parent_id, child1, "child1 must differ from parent");
    assert_ne!(parent_id, child2, "child2 must differ from parent");
    assert_ne!(child1, child2, "each fork must produce a unique SpaceId");
}

/// Test 3: attach_region_adds_binding — attach_region adds a binding to an existing space.
#[tokio::test]
async fn attach_region_adds_binding() {
    let mgr = InMemorySpaceManager::new();
    let space_id = SpaceId::new();

    // Before any attach, the space has no bindings (the entry doesn't exist yet).
    assert!(
        mgr.get_bindings(space_id).is_none(),
        "no entry should exist before first attach"
    );

    let binding1 = ContextBinding::Session("s1".into());
    mgr.attach_region(space_id, binding1.clone())
        .await
        .expect("first attach should succeed");

    let bindings = mgr
        .get_bindings(space_id)
        .expect("space entry must exist after first attach");
    assert_eq!(
        bindings.len(),
        1,
        "space must have one binding after first attach"
    );
    assert_eq!(bindings[0], binding1);

    // Attach a second binding to the same space.
    let binding2 = ContextBinding::MemoryView("mem-view".into());
    mgr.attach_region(space_id, binding2.clone())
        .await
        .expect("second attach should succeed");

    let bindings = mgr
        .get_bindings(space_id)
        .expect("space entry must exist after second attach");
    assert_eq!(
        bindings.len(),
        2,
        "space must have two bindings after second attach"
    );
    assert_eq!(bindings[0], binding1);
    assert_eq!(bindings[1], binding2);
}

/// Test 4: child_overlay_isolated — overlay changes in one space do not affect others.
#[tokio::test]
async fn child_overlay_isolated() {
    let mgr = InMemorySpaceManager::new();
    let parent_id = SpaceId::new();
    let owner = ProcessId::new();

    // Attach a binding to the parent.
    let parent_binding = ContextBinding::Session("parent-session".into());
    mgr.attach_region(parent_id, parent_binding.clone())
        .await
        .expect("attach to parent should succeed");

    // Fork two children from the same parent.
    let child1 = mgr
        .fork_space(parent_id, owner)
        .await
        .expect("fork child1 should succeed");
    let child2 = mgr
        .fork_space(parent_id, owner)
        .await
        .expect("fork child2 should succeed");

    // Attach an additional binding ONLY to child1.
    let child1_binding =
        ContextBinding::Artifact("extra-artifact".into(), fabric::AccessMode::ReadOnly);
    mgr.attach_region(child1, child1_binding.clone())
        .await
        .expect("attach to child1 should succeed");

    // Child1 must now have the parent binding plus its own extra binding.
    let c1_bindings = mgr.get_bindings(child1).expect("child1 must exist");
    assert_eq!(
        c1_bindings.len(),
        2,
        "child1 must have 2 bindings (parent + own)"
    );
    assert_eq!(
        c1_bindings[0], parent_binding,
        "child1 first binding must be parent's"
    );
    assert_eq!(
        c1_bindings[1], child1_binding,
        "child1 second binding must be its own"
    );

    // Child2 must still only have the parent binding (not affected by child1 overlay).
    let c2_bindings = mgr.get_bindings(child2).expect("child2 must exist");
    assert_eq!(
        c2_bindings.len(),
        1,
        "child2 must have only 1 binding (parent only, isolated from child1)"
    );
    assert_eq!(
        c2_bindings[0], parent_binding,
        "child2 binding must still be the parent binding"
    );

    // The parent itself must still have only its original binding.
    let p_bindings = mgr.get_bindings(parent_id).expect("parent must exist");
    assert_eq!(
        p_bindings.len(),
        1,
        "parent must have only its original binding (isolated from child1)"
    );
    assert_eq!(p_bindings[0], parent_binding);
}
