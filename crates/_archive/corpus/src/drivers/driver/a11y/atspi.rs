//! AT-SPI2 accessibility tree driver using the `atspi` crate (zbus-based).

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::CoordType;

use super::A11yDriver;
use crate::driver::types::{Bounds, Element, UiTree};

/// AT-SPI2 accessibility tree driver.
///
/// Connects to the AT-SPI2 D-Bus bus and walks the accessibility tree
/// to produce structured `Element` / `UiTree` results.
///
/// All `atspi` calls are async (zbus); the driver blocks on them internally
/// via a tokio runtime handle.
pub struct AtSpiDriver {
    /// Tokio runtime handle for blocking on async atspi calls.
    handle: tokio::runtime::Handle,
}

impl AtSpiDriver {
    /// Create a new driver using the current tokio runtime handle.
    ///
    /// Must be called from within a tokio runtime context (e.g. inside
    /// `#[tokio::main]` or after `Runtime::enter()`).
    pub fn new() -> Result<Self> {
        let handle = tokio::runtime::Handle::try_current()
            .context("AtSpiDriver requires a tokio runtime context")?;
        Ok(Self { handle })
    }

    /// Synchronously build the UI tree by blocking on async AT-SPI calls.
    fn build_tree_sync(&self) -> Result<UiTree> {
        self.handle.block_on(self.build_tree_async())
    }

    /// Async implementation of tree building.
    async fn build_tree_async(&self) -> Result<UiTree> {
        let conn = atspi::AccessibilityConnection::new()
            .await
            .context("Failed to connect to AT-SPI2 bus. Is at-spi2-registryd running?")?;

        // Get the root accessible from the registry (lists all applications).
        let root = conn
            .root_accessible_on_registry()
            .await
            .context("Failed to get root accessible from AT-SPI2 registry")?;

        // Enumerate application children.
        let app_refs = root
            .get_children()
            .await
            .context("Failed to get desktop children")?;

        if app_refs.is_empty() {
            return Ok(empty_tree("No apps"));
        }

        // Build the tree for the first application.
        let first_app_ref = &app_refs[0];
        let app_accessible = first_app_ref
            .as_accessible_proxy(conn.connection())
            .await
            .context("Failed to build AccessibleProxy for first app")?;

        let app_name = app_accessible.name().await.unwrap_or_default();

        let element = build_element_tree(conn.connection(), &app_accessible).await?;

        Ok(UiTree {
            app_name,
            root: element,
        })
    }
}

impl A11yDriver for AtSpiDriver {
    fn get_tree(&self) -> Result<UiTree> {
        self.build_tree_sync()
    }

    fn get_element_at(&self, x: i32, y: i32) -> Result<Option<Element>> {
        let tree = self.build_tree_sync()?;
        Ok(find_at(&tree.root, x, y))
    }

    fn find_element(&self, description: &str) -> Result<Vec<Element>> {
        let tree = self.build_tree_sync()?;
        Ok(search(&tree.root, &description.to_lowercase()))
    }
}

/// Recursively build an `Element` from an AT-SPI accessible proxy.
///
/// Returns a boxed future to handle recursion (async fn cannot be directly recursive).
fn build_element_tree<'a>(
    conn: &'a atspi::zbus::Connection,
    accessible: &'a AccessibleProxy<'a>,
) -> Pin<Box<dyn Future<Output = Result<Element>> + Send + 'a>> {
    Box::pin(async move {
        let role = accessible.get_role_name().await.unwrap_or_default();
        let name = accessible.name().await.unwrap_or_default();

        // Get text content via the Text interface (if available).
        let text = match accessible.proxies().await {
            Ok(proxies) => match proxies.text().await {
                Ok(tp) => tp.get_text(0, 4096).await.unwrap_or_default(),
                Err(_) => String::new(),
            },
            Err(_) => String::new(),
        };

        // Get bounding box via the Component interface (if available).
        let bounds = match accessible.proxies().await {
            Ok(proxies) => match proxies.component().await {
                Ok(cp) => match cp.get_extents(CoordType::Screen).await {
                    Ok((x, y, w, h)) => Bounds {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    Err(_) => Bounds {
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    },
                },
                Err(_) => Bounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
            },
            Err(_) => Bounds {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
        };

        // Get states.
        let state = match accessible.get_state().await {
            Ok(state_set) => format_state_set(state_set),
            Err(_) => Vec::new(),
        };

        // Get actions via the Action interface (if available).
        let actions = match accessible.proxies().await {
            Ok(proxies) => match proxies.action().await {
                Ok(ap) => {
                    let n = ap.n_actions().await.unwrap_or(0);
                    let mut names = Vec::new();
                    for i in 0..n {
                        if let Ok(name) = ap.get_name(i).await {
                            names.push(name);
                        }
                    }
                    names
                }
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };

        // Recurse into children.
        let child_refs = accessible.get_children().await.unwrap_or_default();
        let mut children = Vec::new();
        for child_ref in &child_refs {
            if child_ref.is_null() {
                continue;
            }
            match child_ref.as_accessible_proxy(conn).await {
                Ok(child_proxy) => match build_element_tree(conn, &child_proxy).await {
                    Ok(elem) => children.push(elem),
                    Err(e) => {
                        tracing::debug!("Skipping child: {e}");
                    }
                },
                Err(e) => {
                    tracing::debug!("Failed to build child proxy: {e}");
                }
            }
        }

        Ok(Element {
            role,
            name,
            text,
            bounds,
            state,
            actions,
            children,
        })
    }) // end Box::pin
}

/// Convert a `StateSet` into a list of string state names.
fn format_state_set(state_set: atspi::StateSet) -> Vec<String> {
    use atspi::State;
    let all_states = [
        State::Invalid,
        State::Active,
        State::Armed,
        State::Busy,
        State::Checked,
        State::Collapsed,
        State::Defunct,
        State::Editable,
        State::Enabled,
        State::Expandable,
        State::Expanded,
        State::Focusable,
        State::Focused,
        State::HasTooltip,
        State::Horizontal,
        State::Iconified,
        State::Modal,
        State::MultiLine,
        State::Multiselectable,
        State::Opaque,
        State::Pressed,
        State::Resizable,
        State::Selectable,
        State::Selected,
        State::Sensitive,
        State::Showing,
        State::SingleLine,
        State::Stale,
        State::Transient,
        State::Truncated,
        State::Vertical,
        State::Visible,
        State::ManagesDescendants,
        State::Indeterminate,
        State::Required,
        State::Animated,
        State::InvalidEntry,
        State::SupportsAutocompletion,
        State::SelectableText,
        State::IsDefault,
        State::Visited,
    ];
    let mut result = Vec::new();
    for s in all_states {
        if state_set.contains(s) {
            result.push(format!("{:?}", s));
        }
    }
    result
}

/// Search for an element whose name, role, or text matches `desc`.
fn search(element: &Element, desc: &str) -> Vec<Element> {
    let mut results = Vec::new();
    if element.name.to_lowercase().contains(desc)
        || element.role.to_lowercase().contains(desc)
        || element.text.to_lowercase().contains(desc)
    {
        results.push(element.clone());
    }
    for child in &element.children {
        results.extend(search(child, desc));
    }
    results
}

/// Find the deepest element at `(x, y)` in the tree.
fn find_at(element: &Element, x: i32, y: i32) -> Option<Element> {
    let b = &element.bounds;
    if b.width > 0
        && b.height > 0
        && x >= b.x
        && x < b.x + b.width
        && y >= b.y
        && y < b.y + b.height
    {
        for child in &element.children {
            if let Some(found) = find_at(child, x, y) {
                return Some(found);
            }
        }
        Some(element.clone())
    } else {
        None
    }
}

/// Create an empty UI tree (used when no applications are found).
fn empty_tree(reason: &str) -> UiTree {
    UiTree {
        app_name: reason.to_string(),
        root: Element {
            role: "frame".into(),
            name: "Empty".into(),
            text: String::new(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            state: Vec::new(),
            actions: Vec::new(),
            children: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::types::Bounds;

    #[test]
    fn test_find_at_hits() {
        let elem = Element {
            role: "push-button".into(),
            name: "OK".into(),
            text: "OK".into(),
            bounds: Bounds {
                x: 10,
                y: 20,
                width: 80,
                height: 30,
            },
            state: vec!["enabled".into()],
            actions: vec!["click".into()],
            children: vec![],
        };
        assert!(find_at(&elem, 50, 35).is_some());
        assert_eq!(find_at(&elem, 50, 35).unwrap().name, "OK");
    }

    #[test]
    fn test_find_at_miss() {
        let elem = Element {
            role: "push-button".into(),
            name: "OK".into(),
            text: "OK".into(),
            bounds: Bounds {
                x: 10,
                y: 20,
                width: 80,
                height: 30,
            },
            state: vec![],
            actions: vec![],
            children: vec![],
        };
        assert!(find_at(&elem, 200, 200).is_none());
    }

    #[test]
    fn test_search_by_name() {
        let root = Element {
            role: "frame".into(),
            name: "Window".into(),
            text: String::new(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            state: vec![],
            actions: vec![],
            children: vec![
                Element {
                    role: "push-button".into(),
                    name: "OK".into(),
                    text: "OK".into(),
                    bounds: Bounds {
                        x: 10,
                        y: 20,
                        width: 80,
                        height: 30,
                    },
                    state: vec![],
                    actions: vec![],
                    children: vec![],
                },
                Element {
                    role: "text-input".into(),
                    name: "Search".into(),
                    text: String::new(),
                    bounds: Bounds {
                        x: 10,
                        y: 100,
                        width: 300,
                        height: 40,
                    },
                    state: vec![],
                    actions: vec![],
                    children: vec![],
                },
            ],
        };
        let results = search(&root, "ok");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "OK");
    }

    #[test]
    fn test_empty_tree() {
        let tree = empty_tree("test");
        assert_eq!(tree.app_name, "test");
        assert!(tree.root.children.is_empty());
    }
}
