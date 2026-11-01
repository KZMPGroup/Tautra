use tautra_proto::kvm::{MouseButton, ServiceInput};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

const VK_PAUSE: u16 = 0x13;
const VK_RSHIFT: u16 = 0xA1;
const VK_MEDIA_FIRST: u16 = 0xA6;
const VK_MEDIA_LAST: u16 = 0xB7;
const XBUTTON1_DATA: u32 = 1;
const XBUTTON2_DATA: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("Windows menolak input dari Tautra (jendela administrator, UAC, atau layar kunci sedang aktif)")]
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PressedKey {
    scan: u16,
    vk: u16,
    extended: bool,
}

pub trait InjectFallback: Send + Sync {
    fn forward(&self, events: Vec<tautra_proto::kvm::ServiceInput>) -> bool;
}

#[derive(Default)]
pub struct Injector {
    keys: Vec<PressedKey>,
    buttons: u8,
    fallback: Option<std::sync::Arc<dyn InjectFallback>>,
}

fn button_bit(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
        MouseButton::X1 => 8,
        MouseButton::X2 => 16,
    }
}

fn button_flags(button: MouseButton, down: bool) -> (MOUSE_EVENT_FLAGS, u32) {
    match (button, down) {
        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1_DATA),
        (MouseButton::X1, false) => (MOUSEEVENTF_XUP, XBUTTON1_DATA),
        (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2_DATA),
        (MouseButton::X2, false) => (MOUSEEVENTF_XUP, XBUTTON2_DATA),
    }
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn normalize(value: i32, origin: i32, size: i32) -> i32 {
    let size = size.max(1) as i64;
    let offset = (value - origin).clamp(0, (size - 1) as i32) as i64;
    ((offset * 65536 + size - 1) / size).min(65535) as i32
}

fn absolute_move(x: i32, y: i32) -> INPUT {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    mouse(
        normalize(x, vx, vw),
        normalize(y, vy, vh),
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

fn key_input(key: PressedKey, down: bool) -> INPUT {
    let use_vk =
        key.scan == 0 || key.vk == VK_PAUSE || (VK_MEDIA_FIRST..=VK_MEDIA_LAST).contains(&key.vk);
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if !use_vk {
        flags |= KEYEVENTF_SCANCODE;
    }
    if key.extended && key.vk != VK_RSHIFT {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(if use_vk { key.vk } else { 0 }),
                wScan: key.scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn cursor() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

fn send(inputs: &[INPUT]) -> Result<(), InjectError> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(InjectError::Blocked)
    }
}

impl Injector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fallback(&mut self, fallback: std::sync::Arc<dyn InjectFallback>) {
        self.fallback = Some(fallback);
    }

    fn route_to_service(&self, check: impl FnOnce() -> bool) -> bool {
        self.fallback.is_some() && check()
    }

    fn dispatch(
        &self,
        inputs: &[INPUT],
        events: Vec<ServiceInput>,
        elevated: bool,
    ) -> Result<(), InjectError> {
        if elevated {
            if let Some(fallback) = &self.fallback {
                if fallback.forward(events.clone()) {
                    return Ok(());
                }
            }
        }
        match send(inputs) {
            Ok(()) => Ok(()),
            Err(e) => match &self.fallback {
                Some(fallback) if !elevated && fallback.forward(events) => Ok(()),
                _ => Err(e),
            },
        }
    }

    pub fn move_to(&mut self, x: i32, y: i32) -> Result<(), InjectError> {
        let elevated = self.route_to_service(|| {
            crate::elevation::point_blocks_injection(x, y)
                || crate::elevation::foreground_blocks_injection()
        });
        self.dispatch(&[absolute_move(x, y)], vec![ServiceInput::Move { x, y }], elevated)
    }

    pub fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        x: i32,
        y: i32,
    ) -> Result<(), InjectError> {
        let bit = button_bit(button);
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        let (flags, data) = button_flags(button, down);
        let elevated = self.route_to_service(|| {
            crate::elevation::point_blocks_injection(x, y)
                || crate::elevation::foreground_blocks_injection()
        });
        self.dispatch(
            &[absolute_move(x, y), mouse(0, 0, data, flags)],
            vec![ServiceInput::Button { button, down, x, y }],
            elevated,
        )
    }

    pub fn forget_button(&mut self, button: MouseButton) {
        self.buttons &= !button_bit(button);
    }

    pub fn wheel(&mut self, delta: i32, horizontal: bool) -> Result<(), InjectError> {
        let flags = if horizontal {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        };
        let elevated = self.route_to_service(|| {
            cursor()
                .map(|(x, y)| crate::elevation::point_blocks_injection(x, y))
                .unwrap_or(false)
                || crate::elevation::foreground_blocks_injection()
        });
        self.dispatch(
            &[mouse(0, 0, delta as u32, flags)],
            vec![ServiceInput::Wheel { delta, horizontal }],
            elevated,
        )
    }

    pub fn key(
        &mut self,
        scan: u16,
        vk: u16,
        extended: bool,
        down: bool,
    ) -> Result<(), InjectError> {
        let key = PressedKey { scan, vk, extended };
        if down {
            if !self.keys.contains(&key) {
                self.keys.push(key);
            }
        } else {
            self.keys.retain(|k| *k != key);
        }
        let elevated = self.route_to_service(crate::elevation::foreground_blocks_injection);
        self.dispatch(
            &[key_input(key, down)],
            vec![ServiceInput::Key { scan, vk, extended, down }],
            elevated,
        )
    }

    pub fn release_all(&mut self) {
        let mut inputs: Vec<INPUT> = self
            .keys
            .drain(..)
            .rev()
            .map(|k| key_input(k, false))
            .collect();
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::X1,
            MouseButton::X2,
        ] {
            if self.buttons & button_bit(button) != 0 {
                let (flags, data) = button_flags(button, false);
                inputs.push(mouse(0, 0, data, flags));
            }
        }
        self.buttons = 0;
        if !inputs.is_empty() {
            let elevated = self.route_to_service(crate::elevation::foreground_blocks_injection);
            if let Err(e) = self.dispatch(&inputs, vec![ServiceInput::ReleaseAll], elevated) {
                tracing::warn!("melepas tombol yang tertahan gagal: {e}");
            }
        }
    }
}
