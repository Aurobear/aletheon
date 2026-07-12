//! Accessibility driver trait and mock implementation.

use crate::drivers::types::{Bounds, Element, UiTree};
use anyhow::Result;

/// Accessibility driver trait — provides structured UI tree access.
pub trait A11yDriver: Send + Sync {
    /// Get the full accessibility UI tree.
    fn get_tree(&self) -> Result<UiTree>;
    /// Get the element at the given screen coordinates.
    fn get_element_at(&self, x: i32, y: i32) -> Result<Option<Element>>;
    /// Find elements matching a description (name, role, or text).
    fn find_element(&self, description: &str) -> Result<Vec<Element>>;
}

/// Mock accessibility driver for testing.
pub struct MockA11yDriver {
    pub tree: UiTree,
}

impl Default for MockA11yDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl MockA11yDriver {
    pub fn new() -> Self {
        Self {
            tree: UiTree {
                app_name: "MockApp".into(),
                root: Element {
                    role: "frame".into(),
                    name: "Mock Window".into(),
                    text: String::new(),
                    bounds: Bounds {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                    state: vec!["showing".into()],
                    actions: vec![],
                    children: vec![
                        Element {
                            role: "push-button".into(),
                            name: "OK".into(),
                            text: "OK".into(),
                            bounds: Bounds {
                                x: 100,
                                y: 200,
                                width: 80,
                                height: 30,
                            },
                            state: vec!["enabled".into()],
                            actions: vec!["click".into()],
                            children: vec![],
                        },
                        Element {
                            role: "text-input".into(),
                            name: "Search".into(),
                            text: String::new(),
                            bounds: Bounds {
                                x: 100,
                                y: 100,
                                width: 300,
                                height: 40,
                            },
                            state: vec!["enabled", "focused"]
                                .into_iter()
                                .map(String::from)
                                .collect(),
                            actions: vec!["focus".into(), "type".into()],
                            children: vec![],
                        },
                    ],
                },
            },
        }
    }
}

impl A11yDriver for MockA11yDriver {
    fn get_tree(&self) -> Result<UiTree> {
        Ok(self.tree.clone())
    }

    fn get_element_at(&self, x: i32, y: i32) -> Result<Option<Element>> {
        fn find_in(element: &Element, x: i32, y: i32) -> Option<Element> {
            let b = &element.bounds;
            if x >= b.x && x < b.x + b.width && y >= b.y && y < b.y + b.height {
                for child in &element.children {
                    if let Some(found) = find_in(child, x, y) {
                        return Some(found);
                    }
                }
                Some(element.clone())
            } else {
                None
            }
        }
        Ok(find_in(&self.tree.root, x, y))
    }

    fn find_element(&self, description: &str) -> Result<Vec<Element>> {
        let desc = description.to_lowercase();
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
        Ok(search(&self.tree.root, &desc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_get_tree() {
        let driver = MockA11yDriver::new();
        let tree = driver.get_tree().unwrap();
        assert_eq!(tree.app_name, "MockApp");
        assert_eq!(tree.root.children.len(), 2);
    }

    #[test]
    fn test_mock_find_element() {
        let driver = MockA11yDriver::new();
        let results = driver.find_element("ok").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "OK");
    }

    #[test]
    fn test_mock_get_element_at() {
        let driver = MockA11yDriver::new();
        // Hit within OK button bounds (100..180, 200..230)
        let elem = driver.get_element_at(120, 210).unwrap();
        assert!(elem.is_some());
        assert_eq!(elem.unwrap().name, "OK");
    }

    #[test]
    fn test_get_element_at_miss() {
        let driver = MockA11yDriver::new();
        // Outside any element
        let elem = driver.get_element_at(0, 0).unwrap();
        // (0,0) is inside the root frame, so returns root
        assert!(elem.is_some());
        assert_eq!(elem.unwrap().name, "Mock Window");
    }

    #[test]
    fn test_find_element_by_role() {
        let driver = MockA11yDriver::new();
        let results = driver.find_element("text-input").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Search");
    }

    #[test]
    fn test_find_element_no_match() {
        let driver = MockA11yDriver::new();
        let results = driver.find_element("nonexistent").unwrap();
        assert!(results.is_empty());
    }
}

#[cfg(feature = "a11y")]
pub mod atspi;

#[cfg(feature = "a11y")]
pub use atspi::AtSpiDriver;
