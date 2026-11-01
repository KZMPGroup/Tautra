use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    #[error("hotkey kosong")]
    Empty,
    #[error("tombol tidak dikenal: {0}")]
    UnknownKey(String),
    #[error("hotkey harus punya satu tombol utama selain Ctrl/Alt/Shift/Win")]
    MissingKey,
    #[error("hotkey hanya boleh punya satu tombol utama")]
    TooManyKeys,
}

const NAMED_KEYS: &[(&str, u16)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Pause", 0x13),
    ("CapsLock", 0x14),
    ("Esc", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("PrintScreen", 0x2C),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("NumLock", 0x90),
    ("ScrollLock", 0x91),
    ("Numpad0", 0x60),
    ("Numpad1", 0x61),
    ("Numpad2", 0x62),
    ("Numpad3", 0x63),
    ("Numpad4", 0x64),
    ("Numpad5", 0x65),
    ("Numpad6", 0x66),
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("NumpadMultiply", 0x6A),
    ("NumpadAdd", 0x6B),
    ("NumpadSubtract", 0x6D),
    ("NumpadDecimal", 0x6E),
    ("NumpadDivide", 0x6F),
    (";", 0xBA),
    ("=", 0xBB),
    (",", 0xBC),
    ("-", 0xBD),
    (".", 0xBE),
    ("/", 0xBF),
    ("`", 0xC0),
    ("[", 0xDB),
    ("\\", 0xDC),
    ("]", 0xDD),
    ("'", 0xDE),
];

const ALIASES: &[(&str, u16)] = &[
    ("Escape", 0x1B),
    ("Return", 0x0D),
    ("Del", 0x2E),
    ("Ins", 0x2D),
    ("PgUp", 0x21),
    ("PgDn", 0x22),
    ("ScrLk", 0x91),
    ("PrtSc", 0x2C),
];

pub fn key_from_name(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    NAMED_KEYS
        .iter()
        .chain(ALIASES)
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, vk)| *vk)
}

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, v)| *v == vk)
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| format!("0x{vk:02X}")),
    }
}

impl Hotkey {
    pub fn parse(text: &str) -> Result<Self, HotkeyError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HotkeyError::Empty);
        }
        let mut hotkey = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0,
        };
        let parts: Vec<&str> = if text == "+" {
            vec!["+"]
        } else {
            text.split('+').map(str::trim).collect()
        };
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hotkey.ctrl = true,
                "alt" => hotkey.alt = true,
                "shift" => hotkey.shift = true,
                "win" | "meta" | "super" => hotkey.win = true,
                "" => return Err(HotkeyError::UnknownKey(text.to_string())),
                _ => {
                    if hotkey.vk != 0 {
                        return Err(HotkeyError::TooManyKeys);
                    }
                    hotkey.vk = key_from_name(part)
                        .ok_or_else(|| HotkeyError::UnknownKey(part.to_string()))?;
                }
            }
        }
        if hotkey.vk == 0 {
            return Err(HotkeyError::MissingKey);
        }
        Ok(hotkey)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        parts.push(key_name(self.vk));
        f.write_str(&parts.join("+"))
    }
}
