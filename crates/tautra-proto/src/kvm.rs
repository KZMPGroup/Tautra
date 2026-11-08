use serde::{Deserialize, Serialize};

use crate::share::FileOffer;
use crate::DeviceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamKind {
    Kvm,
    Share,
    ClipFetch,
    FileGet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHeader {
    pub kind: StreamKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale_percent: u32,
    pub width_mm: u32,
    pub height_mm: u32,
    pub primary: bool,
}

const UNIT_MM: f64 = 25.4 / 96.0;
const MIN_DIAGONAL_MM: f64 = 150.0;
const MAX_DIAGONAL_MM: f64 = 1600.0;
const ASPECT_TOLERANCE: f64 = 0.12;

impl MonitorInfo {
    pub fn dpi_scale(&self) -> f64 {
        self.scale_percent.max(25) as f64 / 100.0
    }

    pub fn physical_scale(&self) -> Option<f64> {
        if self.width_mm == 0 || self.height_mm == 0 || self.width <= 0 || self.height <= 0 {
            return None;
        }
        let (w, h) = (self.width_mm as f64, self.height_mm as f64);
        let diagonal = (w * w + h * h).sqrt();
        if !(MIN_DIAGONAL_MM..=MAX_DIAGONAL_MM).contains(&diagonal) {
            return None;
        }
        let aspect_px = self.width as f64 / self.height as f64;
        let aspect_mm = w / h;
        if (aspect_px / aspect_mm - 1.0).abs() > ASPECT_TOLERANCE {
            return None;
        }
        Some(self.width as f64 * UNIT_MM / w)
    }

    pub fn layout_scale(&self) -> f64 {
        self.physical_scale().unwrap_or_else(|| self.dpi_scale())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DisplaySet {
    pub monitors: Vec<MonitorInfo>,
}

impl DisplaySet {
    pub fn primary(&self) -> Option<&MonitorInfo> {
        self.monitors
            .iter()
            .find(|m| m.primary)
            .or_else(|| self.monitors.first())
    }

    pub fn scale(&self) -> f64 {
        self.primary().map(MonitorInfo::layout_scale).unwrap_or(1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePosition {
    pub device_id: DeviceId,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairLayout {
    pub revision: u64,
    pub input_host: DeviceId,
    pub positions: Vec<DevicePosition>,
}

impl PairLayout {
    pub fn position(&self, device_id: &DeviceId) -> Option<(i32, i32)> {
        self.positions
            .iter()
            .find(|p| &p.device_id == device_id)
            .map(|p| (p.x, p.y))
    }

    pub fn is_newer_than(&self, other: &PairLayout) -> bool {
        (self.revision, self.input_host) > (other.revision, other.input_host)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KvmMessage {
    Hello {
        displays: DisplaySet,
        layout: Option<PairLayout>,
        portable: bool,
    },
    Displays(DisplaySet),
    Layout(PairLayout),
    Enter {
        x: i32,
        y: i32,
        edge: Option<CrossEdge>,
    },
    Leave {
        x: i32,
        y: i32,
        edge: Option<CrossEdge>,
    },
    Button {
        button: MouseButton,
        down: bool,
        x: i32,
        y: i32,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Key {
        scan: u16,
        vk: u16,
        extended: bool,
        down: bool,
    },
    ReleaseAll,
    DragProbe,
    DragResult {
        files: bool,
    },
    DragHold,
    DragFiles(FileOffer),
    DragReturn {
        offer_id: u64,
    },
    LayoutGuide(Option<Vec<DevicePosition>>),
    Prefs {
        crossing_animation: bool,
        crossing_style: u8,
        changed: bool,
    },
}

pub const DATAGRAM_MOUSE_MOVE: u8 = 1;
pub const MOUSE_MOVE_LEN: usize = 13;

pub fn encode_mouse_move(seq: u32, x: i32, y: i32) -> [u8; MOUSE_MOVE_LEN] {
    let mut buf = [0u8; MOUSE_MOVE_LEN];
    buf[0] = DATAGRAM_MOUSE_MOVE;
    buf[1..5].copy_from_slice(&seq.to_le_bytes());
    buf[5..9].copy_from_slice(&x.to_le_bytes());
    buf[9..13].copy_from_slice(&y.to_le_bytes());
    buf
}

pub fn decode_mouse_move(buf: &[u8]) -> Option<(u32, i32, i32)> {
    if buf.len() != MOUSE_MOVE_LEN || buf[0] != DATAGRAM_MOUSE_MOVE {
        return None;
    }
    let seq = u32::from_le_bytes(buf[1..5].try_into().ok()?);
    let x = i32::from_le_bytes(buf[5..9].try_into().ok()?);
    let y = i32::from_le_bytes(buf[9..13].try_into().ok()?);
    Some((seq, x, y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceInput {
    Move { x: i32, y: i32 },
    Button { button: MouseButton, down: bool, x: i32, y: i32 },
    Wheel { delta: i32, horizontal: bool },
    Key { scan: u16, vk: u16, extended: bool, down: bool },
    ReleaseAll,
}
