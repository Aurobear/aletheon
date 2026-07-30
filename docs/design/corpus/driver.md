> New document — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# Driver Subsystem

> Optional Linux desktop adapters for display, input, OCR, accessibility, and sandbox primitives.

**Crate:** `corpus`
**Module:** `crates/corpus/src/drivers/`
**Last updated:** 2026-07-30

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| DriverFactory | ✅ Implemented | `crates/corpus/src/drivers/factory.rs` | Feature-aware adapter construction |
| InputDriver (uinput) | 🔶 Optional | `crates/corpus/src/drivers/input/` | Linux uinput, behind `input` feature |
| Display/clipboard/window | 🔶 Optional | `crates/corpus/src/drivers/display/` | Linux X11/DRM adapters, behind `display` feature |
| A11yDriver (AT-SPI) | 🔶 Optional | `crates/corpus/src/drivers/a11y/` | Behind `a11y` feature |
| OcrDriver (Tesseract) | 🔶 Optional | `crates/corpus/src/drivers/ocr/` | Behind `ocr-tesseract` feature |
| ProcessDriver / IoDriver | Removed | — | Empty public placeholders were removed; host operations belong to `platform` |
| SandboxDriver | 🔶 Optional | `crates/corpus/src/drivers/sandbox_driver/` | Low-level primitives behind `sandbox-primitives` |

---

## 1. Architecture

The driver subsystem provides safe Rust bindings to Linux kernel interfaces and system services. Each driver module is feature-gated by hardware capability, allowing the crate to compile on systems without specific hardware.

```
DriverFactory
    ├── try_input()      → UinputDriver        (feature = "input")
    ├── try_display()    → X11DisplayDriver     (feature = "display")
    │                    → FramebufferDriver    (fallback)
    ├── try_a11y()       → AtSpiDriver          (feature = "a11y")
    ├── try_ocr()        → TesseractOcrDriver   (feature = "ocr-tesseract")
    ├── try_window()     → EwmhWindowManager    (feature = "display")
    └── try_clipboard()  → X11ClipboardDriver   (feature = "display")
```

## 2. Display Driver

**Feature gate:** `display`

### 2.1 X11DisplayDriver

Captures screenshots via X11 protocol. Falls back to framebuffer (`/dev/fb0`) for headless systems.

- Screenshot capture returns `Image` (RGB bytes, row-major)
- Supports multi-monitor via Xinerama

### 2.2 WindowManager (EWMH)

EWMH-compliant window management:
- List windows, get active window
- Focus, move, resize windows
- Get window title and geometry

### 2.3 ClipboardDriver

X11 clipboard operations:
- `get_text()` — read clipboard content
- `set_text(text)` — write clipboard content

## 3. Input Driver

**Feature gate:** `input`

### 3.1 UinputDriver

Creates a virtual input device via Linux uinput (`/dev/uinput`):
- `move_mouse(x, y)` — absolute mouse positioning
- `click(button)` — mouse click
- `scroll(direction)` — scroll wheel
- `type_text(text)` — keyboard text input
- `press_key(key)` / `release_key(key)` — individual key events

**Core types** (from `driver/types.rs`):
- `MouseButton` — Left, Right, Middle
- `ScrollDirection` — Up, Down, Left, Right
- `Key` — full keyboard key enum (A-Z, 0-9, function keys, modifiers)

## 4. OCR Driver

**Feature gate:** `ocr-tesseract`

### 4.1 TesseractOcrDriver

OCR via Tesseract library:
- `recognize(image) -> OcrResult` — extract text from image
- Returns word-level bounding boxes and confidence scores

**Core types** (from `driver/types.rs`):
- `OcrResult` — text + words with bounds and confidence
- `OcrWord` — individual word with `Bounds` and confidence

## 5. Accessibility Driver

**Feature gate:** `a11y`

### 5.1 AtSpiDriver

AT-SPI2 accessibility tree via D-Bus session bus:
- `get_tree() -> UiTree` — full accessibility tree
- `find_elements(query)` — search by role/name
- Requires `DBUS_SESSION_BUS_ADDRESS` environment variable

**Core types** (from `driver/types.rs`):
- `Element` — UI element with role, name, text, bounds, state, actions, children
- `UiTree` — root element + app name
- `Bounds` — x, y, width, height

## 6. Observation Model

The driver subsystem supports a layered observation strategy:

```rust
enum Observation {
    AccessibilityTree(UiTree),  // Best: structured UI tree (AT-SPI)
    OcrFallback(OcrResult),     // Good: OCR text extraction
    ScreenshotOnly(Image),       // Minimum: raw screenshot
}
```

Priority: AccessibilityTree > OcrFallback > ScreenshotOnly. The `computer` module (feature-gated behind `input + display + a11y`) orchestrates the observation pipeline.

## 7. Process & I/O Drivers

### 7.1 Host process and filesystem operations

These capabilities are owned by `crates/platform/src/process.rs` and
`crates/platform/src/filesystem.rs`; Corpus no longer publishes empty driver
names for them.

## 8. Sandbox Driver

**Feature gate:** `sandbox-primitives`

Provides low-level sandbox primitives used by the sandbox execution layer (see [sandbox.md](sandbox.md)).

## 9. Feature Gates

| Feature | Enables | Required Hardware |
|---------|---------|-------------------|
| `input` | UinputDriver | `/dev/uinput` |
| `display` | X11DisplayDriver, FramebufferDriver, WindowManager, Clipboard | X11 or `/dev/fb0` |
| `a11y` | AtSpiDriver | D-Bus session bus |
| `ocr-tesseract` | TesseractOcrDriver | Tesseract library |
| `sandbox-primitives` | SandboxDriver | Linux namespace support |

## 10. Implementation Notes

**Code location:** `crates/corpus/src/drivers/`

**Key design decisions:**
- All drivers are optional — `DriverFactory::try_*()` returns `Option<Box<dyn Trait>>`
- Feature gates allow compilation on systems without specific hardware
- `Image::to_base64_png()` converts screenshots for LLM vision APIs
- `Observation` enum provides a unified view across different input modalities
