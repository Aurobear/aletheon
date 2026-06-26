use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::Mutex;
use anyhow::{Result, Context};
use crate::r#impl::driver::types::{Key, MouseButton, ScrollDirection};
use super::InputDriver;

// uinput ioctl constants
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
const UI_SET_RELBIT: u64 = 0x40045566;
const UI_SET_ABSBIT: u64 = 0x40045568;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_ABS_SETUP: u64 = 0x400C5503;

// Event types
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const EV_SYN: u16 = 0;

// Absolute axes
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;

// Relative axes
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;

// Button codes
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

// Sync
const SYN_REPORT: u16 = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UinputAbsSetup {
    code: u16,
    _pad: u16,
    absinfo: AbsInfo,
}

/// uinput virtual input device driver
pub struct UinputDriver {
    file: Mutex<File>,
}

impl UinputDriver {
    /// Create a uinput virtual device
    pub fn create() -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .context("Failed to open /dev/uinput. Need root or input group.")?;

        let fd = file.as_raw_fd();

        // Enable event types
        unsafe {
            libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_uint);
            libc::ioctl(fd, UI_SET_EVBIT, EV_REL as libc::c_uint);
            libc::ioctl(fd, UI_SET_EVBIT, EV_ABS as libc::c_uint);
        }

        // Enable keys
        for key_code in Self::all_key_codes() {
            unsafe { libc::ioctl(fd, UI_SET_KEYBIT, key_code as libc::c_uint); }
        }

        // Enable relative axes
        for rel in [REL_X, REL_Y, REL_WHEEL] {
            unsafe { libc::ioctl(fd, UI_SET_RELBIT, rel as libc::c_uint); }
        }

        // Enable and configure absolute axes (for touch/absolute positioning)
        unsafe { libc::ioctl(fd, UI_SET_ABSBIT, ABS_X as libc::c_uint); }
        unsafe { libc::ioctl(fd, UI_SET_ABSBIT, ABS_Y as libc::c_uint); }

        for axis in [ABS_X, ABS_Y] {
            let abs_setup = UinputAbsSetup {
                code: axis,
                _pad: 0,
                absinfo: AbsInfo {
                    value: 0,
                    minimum: 0,
                    maximum: 32767,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                },
            };
            unsafe { libc::ioctl(fd, UI_ABS_SETUP, &abs_setup); }
        }

        // Setup device
        let setup = UinputSetup {
            id: InputId {
                bustype: 0x03, // BUS_USB
                vendor: 0x1234,
                product: 0x5678,
                version: 1,
            },
            name: {
                let mut name = [0u8; 80];
                let s = b"aletheon-virtual-input";
                name[..s.len()].copy_from_slice(s);
                name
            },
            ff_effects_max: 0,
        };

        unsafe {
            libc::ioctl(fd, UI_DEV_CREATE, &setup);
        }

        Ok(Self { file: Mutex::new(file) })
    }

    fn all_key_codes() -> Vec<u16> {
        // KEY_* codes from linux/input-event-codes.h
        vec![
            30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, // A-P
            16, 19, 31, 20, 22, 47, 17, 45, 21, 44, // Q-Z
            11, 2, 3, 4, 5, 6, 7, 8, 9, 10, // 0-9
            28, 57, 15, 1, 14, // Enter, Space, Tab, Escape, Backspace
            111, 103, 108, 105, 106, // Delete, Up, Down, Left, Right
            102, 107, 104, 109, // Home, End, PageUp, PageDown
            59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88, // F1-F12
            29, 56, 42, 125, // Ctrl, Alt, Shift, Super
        ]
    }

    fn key_to_code(key: Key) -> u16 {
        match key {
            Key::A => 30, Key::B => 48, Key::C => 46, Key::D => 32,
            Key::E => 18, Key::F => 33, Key::G => 34, Key::H => 35,
            Key::I => 23, Key::J => 36, Key::K => 37, Key::L => 38,
            Key::M => 50, Key::N => 49, Key::O => 24, Key::P => 25,
            Key::Q => 16, Key::R => 19, Key::S => 31, Key::T => 20,
            Key::U => 22, Key::V => 47, Key::W => 17, Key::X => 45,
            Key::Y => 21, Key::Z => 44,
            Key::Num0 => 11, Key::Num1 => 2, Key::Num2 => 3, Key::Num3 => 4,
            Key::Num4 => 5, Key::Num5 => 6, Key::Num6 => 7, Key::Num7 => 8,
            Key::Num8 => 9, Key::Num9 => 10,
            Key::Enter => 28, Key::Space => 57, Key::Tab => 15,
            Key::Escape => 1, Key::Backspace => 14, Key::Delete => 111,
            Key::Up => 103, Key::Down => 108, Key::Left => 105, Key::Right => 106,
            Key::Home => 102, Key::End => 107, Key::PageUp => 104, Key::PageDown => 109,
            Key::F1 => 59, Key::F2 => 60, Key::F3 => 61, Key::F4 => 62,
            Key::F5 => 63, Key::F6 => 64, Key::F7 => 65, Key::F8 => 66,
            Key::F9 => 67, Key::F10 => 68, Key::F11 => 87, Key::F12 => 88,
            Key::Ctrl => 29, Key::Alt => 56, Key::Shift => 42, Key::Super => 125,
        }
    }

    fn char_to_keycode(c: char) -> Option<(Key, bool)> {
        match c {
            'a' => Some((Key::A, false)), 'b' => Some((Key::B, false)),
            'c' => Some((Key::C, false)), 'd' => Some((Key::D, false)),
            'e' => Some((Key::E, false)), 'f' => Some((Key::F, false)),
            'g' => Some((Key::G, false)), 'h' => Some((Key::H, false)),
            'i' => Some((Key::I, false)), 'j' => Some((Key::J, false)),
            'k' => Some((Key::K, false)), 'l' => Some((Key::L, false)),
            'm' => Some((Key::M, false)), 'n' => Some((Key::N, false)),
            'o' => Some((Key::O, false)), 'p' => Some((Key::P, false)),
            'q' => Some((Key::Q, false)), 'r' => Some((Key::R, false)),
            's' => Some((Key::S, false)), 't' => Some((Key::T, false)),
            'u' => Some((Key::U, false)), 'v' => Some((Key::V, false)),
            'w' => Some((Key::W, false)), 'x' => Some((Key::X, false)),
            'y' => Some((Key::Y, false)), 'z' => Some((Key::Z, false)),
            'A' => Some((Key::A, true)), 'B' => Some((Key::B, true)),
            'C' => Some((Key::C, true)), 'D' => Some((Key::D, true)),
            'E' => Some((Key::E, true)), 'F' => Some((Key::F, true)),
            'G' => Some((Key::G, true)), 'H' => Some((Key::H, true)),
            'I' => Some((Key::I, true)), 'J' => Some((Key::J, true)),
            'K' => Some((Key::K, true)), 'L' => Some((Key::L, true)),
            'M' => Some((Key::M, true)), 'N' => Some((Key::N, true)),
            'O' => Some((Key::O, true)), 'P' => Some((Key::P, true)),
            'Q' => Some((Key::Q, true)), 'R' => Some((Key::R, true)),
            'S' => Some((Key::S, true)), 'T' => Some((Key::T, true)),
            'U' => Some((Key::U, true)), 'V' => Some((Key::V, true)),
            'W' => Some((Key::W, true)), 'X' => Some((Key::X, true)),
            'Y' => Some((Key::Y, true)), 'Z' => Some((Key::Z, true)),
            '0' => Some((Key::Num0, false)), '1' => Some((Key::Num1, false)),
            '2' => Some((Key::Num2, false)), '3' => Some((Key::Num3, false)),
            '4' => Some((Key::Num4, false)), '5' => Some((Key::Num5, false)),
            '6' => Some((Key::Num6, false)), '7' => Some((Key::Num7, false)),
            '8' => Some((Key::Num8, false)), '9' => Some((Key::Num9, false)),
            ' ' => Some((Key::Space, false)),
            '\n' => Some((Key::Enter, false)),
            '\t' => Some((Key::Tab, false)),
            _ => None,
        }
    }

    fn write_event(&self, type_: u16, code: u16, value: i32) -> Result<()> {
        let event = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const _ as *const u8,
                std::mem::size_of::<InputEvent>(),
            )
        };
        let mut file = self.file.lock().unwrap();
        file.write_all(bytes).context("Failed to write input event")?;
        Ok(())
    }
}

impl InputDriver for UinputDriver {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<()> {
        let btn_code = match button {
            MouseButton::Left => BTN_LEFT,
            MouseButton::Right => BTN_RIGHT,
            MouseButton::Middle => BTN_MIDDLE,
        };
        // Move to absolute position
        self.write_event(EV_ABS, ABS_X, x)?;
        self.write_event(EV_ABS, ABS_Y, y)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        // Press and release
        self.write_event(EV_KEY, btn_code, 1)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        self.write_event(EV_KEY, btn_code, 0)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            if let Some((key, needs_shift)) = Self::char_to_keycode(ch) {
                let code = Self::key_to_code(key);
                if needs_shift {
                    self.write_event(EV_KEY, Self::key_to_code(Key::Shift), 1)?;
                    self.write_event(EV_SYN, SYN_REPORT, 0)?;
                }
                self.write_event(EV_KEY, code, 1)?;
                self.write_event(EV_SYN, SYN_REPORT, 0)?;
                self.write_event(EV_KEY, code, 0)?;
                self.write_event(EV_SYN, SYN_REPORT, 0)?;
                if needs_shift {
                    self.write_event(EV_KEY, Self::key_to_code(Key::Shift), 0)?;
                    self.write_event(EV_SYN, SYN_REPORT, 0)?;
                }
            }
        }
        Ok(())
    }

    fn hotkey(&self, keys: &[Key]) -> Result<()> {
        // Press all keys, then release in reverse
        for key in keys {
            self.write_event(EV_KEY, Self::key_to_code(*key), 1)?;
            self.write_event(EV_SYN, SYN_REPORT, 0)?;
        }
        for key in keys.iter().rev() {
            self.write_event(EV_KEY, Self::key_to_code(*key), 0)?;
            self.write_event(EV_SYN, SYN_REPORT, 0)?;
        }
        Ok(())
    }

    fn scroll(&self, _x: i32, _y: i32, direction: ScrollDirection, amount: i32) -> Result<()> {
        let value = match direction {
            ScrollDirection::Up => amount,
            ScrollDirection::Down => -amount,
            ScrollDirection::Left => amount,
            ScrollDirection::Right => -amount,
        };
        let axis = match direction {
            ScrollDirection::Up | ScrollDirection::Down => REL_WHEEL,
            ScrollDirection::Left | ScrollDirection::Right => 0x06, // REL_HWHEEL
        };
        self.write_event(EV_REL, axis, value)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }

    fn drag(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<()> {
        // Move to start position
        self.write_event(EV_ABS, ABS_X, x1)?;
        self.write_event(EV_ABS, ABS_Y, y1)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        // Press left button
        self.write_event(EV_KEY, BTN_LEFT, 1)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        // Move to end position
        self.write_event(EV_ABS, ABS_X, x2)?;
        self.write_event(EV_ABS, ABS_Y, y2)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        // Release left button
        self.write_event(EV_KEY, BTN_LEFT, 0)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }
}
