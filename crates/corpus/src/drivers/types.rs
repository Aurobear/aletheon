use serde::{Deserialize, Serialize};

/// Mouse button
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Scroll direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Keyboard key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Enter,
    Space,
    Tab,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Ctrl,
    Alt,
    Shift,
    Super,
}

// Re-exported from base (Tier 2c) for backward compatibility.
pub use fabric::types::vision::{Bounds, Image};

/// UI element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub role: String,
    pub name: String,
    pub text: String,
    pub bounds: Bounds,
    pub state: Vec<String>,
    pub actions: Vec<String>,
    pub children: Vec<Element>,
}

/// UI tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiTree {
    pub root: Element,
    pub app_name: String,
}

/// OCR result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    pub words: Vec<OcrWord>,
}

/// OCR word
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrWord {
    pub text: String,
    pub bounds: Bounds,
    pub confidence: f32,
}

/// Smart observation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Observation {
    AccessibilityTree(UiTree),
    OcrFallback(OcrResult),
    ScreenshotOnly(Image),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_creation() {
        let img = Image {
            width: 2,
            height: 2,
            data: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128],
        };
        assert_eq!(img.data.len(), 12); // 2*2*3
    }

    #[test]
    fn test_to_base64_png() {
        let img = Image {
            width: 2,
            height: 2,
            data: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let (media_type, b64) = img.to_base64_png().unwrap();
        assert_eq!(media_type, "image/png");
        assert!(!b64.is_empty());
        // Verify it's valid base64
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .unwrap();
        // PNG magic bytes
        assert_eq!(&decoded[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_element_bounds() {
        let elem = Element {
            role: "push-button".into(),
            name: "OK".into(),
            text: "OK".into(),
            bounds: Bounds {
                x: 10,
                y: 20,
                width: 100,
                height: 30,
            },
            state: vec!["enabled".into()],
            actions: vec!["click".into()],
            children: vec![],
        };
        assert_eq!(elem.bounds.x, 10);
    }
}
