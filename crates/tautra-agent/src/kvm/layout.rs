use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tautra_common::paths;
use tautra_proto::kvm::{DevicePosition, DisplaySet, PairLayout};
use uuid::Uuid;

pub const MAX_COORDINATE: i32 = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPosition {
    device_id: Uuid,
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredLayout {
    peer: Uuid,
    revision: u64,
    input_host: Uuid,
    positions: Vec<StoredPosition>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LayoutFile {
    #[serde(default)]
    layouts: Vec<StoredLayout>,
}

pub struct LayoutStore {
    path: Option<PathBuf>,
    file: LayoutFile,
}

impl LayoutStore {
    pub fn load() -> (Self, Option<String>) {
        let path = match paths::layouts_file() {
            Ok(p) => p,
            Err(e) => {
                return (
                    Self {
                        path: None,
                        file: LayoutFile::default(),
                    },
                    Some(format!("Lokasi layouts.toml tidak tersedia: {e}")),
                )
            }
        };
        let (file, warning) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<LayoutFile>(&text) {
                Ok(file) => (file, None),
                Err(e) => (
                    LayoutFile::default(),
                    Some(format!(
                        "layouts.toml rusak ({e}); layout layar diatur ulang ke default."
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (LayoutFile::default(), None),
            Err(e) => (
                LayoutFile::default(),
                Some(format!("layouts.toml tidak bisa dibaca: {e}")),
            ),
        };
        (
            Self {
                path: Some(path),
                file,
            },
            warning,
        )
    }

    pub fn get(&self, peer: Uuid) -> Option<PairLayout> {
        self.file
            .layouts
            .iter()
            .find(|l| l.peer == peer)
            .map(|l| PairLayout {
                revision: l.revision,
                input_host: *l.input_host.as_bytes(),
                positions: l
                    .positions
                    .iter()
                    .map(|p| DevicePosition {
                        device_id: *p.device_id.as_bytes(),
                        x: p.x,
                        y: p.y,
                    })
                    .collect(),
            })
    }

    pub fn put(&mut self, peer: Uuid, layout: &PairLayout) -> Result<(), String> {
        let stored = StoredLayout {
            peer,
            revision: layout.revision,
            input_host: Uuid::from_bytes(layout.input_host),
            positions: layout
                .positions
                .iter()
                .map(|p| StoredPosition {
                    device_id: Uuid::from_bytes(p.device_id),
                    x: p.x,
                    y: p.y,
                })
                .collect(),
        };
        match self.file.layouts.iter_mut().find(|l| l.peer == peer) {
            Some(existing) => *existing = stored,
            None => self.file.layouts.push(stored),
        }
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| "lokasi layouts.toml tidak tersedia".to_string())?;
        let text = toml::to_string_pretty(&self.file).map_err(|e| e.to_string())?;
        paths::write_atomic(path, text.as_bytes())
            .map_err(|e| format!("gagal menulis {}: {e}", path.display()))
    }
}

pub fn is_valid(layout: &PairLayout, a: Uuid, b: Uuid) -> bool {
    let ids = [*a.as_bytes(), *b.as_bytes()];
    ids.contains(&layout.input_host)
        && ids.iter().all(|id| layout.position(id).is_some())
        && layout
            .positions
            .iter()
            .all(|p| p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE)
}

fn bounds(displays: &DisplaySet) -> (f64, f64, f64, f64) {
    let scale = displays.scale();
    displays.monitors.iter().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(l, t, r, b), m| {
            (
                l.min(m.x as f64 / scale),
                t.min(m.y as f64 / scale),
                r.max((m.x + m.width) as f64 / scale),
                b.max((m.y + m.height) as f64 / scale),
            )
        },
    )
}

pub struct DeviceSpec<'a> {
    pub id: Uuid,
    pub displays: &'a DisplaySet,
    pub portable: bool,
}

pub fn default_host(a: &DeviceSpec, b: &DeviceSpec) -> Uuid {
    match (a.portable, b.portable) {
        (false, true) => a.id,
        (true, false) => b.id,
        _ => a.id.min(b.id),
    }
}

pub fn default_positions(host: &DeviceSpec, other: &DeviceSpec) -> Vec<DevicePosition> {
    let mut positions = vec![DevicePosition {
        device_id: *host.id.as_bytes(),
        x: 0,
        y: 0,
    }];
    let other_pos = if host.displays.monitors.is_empty() || other.displays.monitors.is_empty() {
        (0, 0)
    } else {
        let (_, ht, hr, hb) = bounds(host.displays);
        let (ol, ot, _, ob) = bounds(other.displays);
        let y = if host.portable != other.portable {
            hb - ob
        } else {
            (ht + hb) / 2.0 - (ot + ob) / 2.0
        };
        ((hr - ol).round() as i32, y.round() as i32)
    };
    positions.push(DevicePosition {
        device_id: *other.id.as_bytes(),
        x: other_pos.0,
        y: other_pos.1,
    });
    positions.sort_by_key(|p| p.device_id);
    positions
}

pub fn default_layout(a: &DeviceSpec, b: &DeviceSpec, revision: u64) -> PairLayout {
    let host_id = default_host(a, b);
    let (host, other) = if host_id == a.id { (a, b) } else { (b, a) };
    PairLayout {
        revision,
        input_host: *host_id.as_bytes(),
        positions: default_positions(host, other),
    }
}

const MIN_OVERLAP_RATIO: f64 = 0.15;
const TOUCH_TOLERANCE: f64 = 1.0;

#[derive(Clone, Copy)]
struct Area {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Area {
    fn of(displays: &DisplaySet, (x, y): (i32, i32)) -> Option<Self> {
        if displays.monitors.is_empty() {
            return None;
        }
        let (l, t, r, b) = bounds(displays);
        Some(Self {
            left: x as f64 + l,
            top: y as f64 + t,
            right: x as f64 + r,
            bottom: y as f64 + b,
        })
    }

    fn width(&self) -> f64 {
        self.right - self.left
    }

    fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

fn touching(m: &Area, a: &Area) -> bool {
    let need_y = m.height().min(a.height()) * MIN_OVERLAP_RATIO - TOUCH_TOLERANCE;
    let need_x = m.width().min(a.width()) * MIN_OVERLAP_RATIO - TOUCH_TOLERANCE;
    let side = ((m.left - a.right).abs() <= TOUCH_TOLERANCE
        || (m.right - a.left).abs() <= TOUCH_TOLERANCE)
        && overlap(m.top, m.bottom, a.top, a.bottom) >= need_y;
    let stacked = ((m.top - a.bottom).abs() <= TOUCH_TOLERANCE
        || (m.bottom - a.top).abs() <= TOUCH_TOLERANCE)
        && overlap(m.left, m.right, a.left, a.right) >= need_x;
    side || stacked
}

fn along(start: f64, size: f64, anchor_start: f64, anchor_size: f64) -> f64 {
    let min_overlap = size.min(anchor_size) * MIN_OVERLAP_RATIO;
    let lo = anchor_start - size + min_overlap;
    let hi = anchor_start + anchor_size - min_overlap;
    start.clamp(lo.min(hi), hi.max(lo))
}

fn snap(m: &Area, a: &Area) -> (f64, f64) {
    let cx = (m.left + m.right) / 2.0 - (a.left + a.right) / 2.0;
    let cy = (m.top + m.bottom) / 2.0 - (a.top + a.bottom) / 2.0;
    let nx = cx / ((a.width() + m.width()) / 2.0).max(1.0);
    let ny = cy / ((a.height() + m.height()) / 2.0).max(1.0);
    let (left, top) = if nx.abs() >= ny.abs() {
        let left = if nx >= 0.0 { a.right } else { a.left - m.width() };
        (left, along(m.top, m.height(), a.top, a.height()))
    } else {
        let top = if ny >= 0.0 { a.bottom } else { a.top - m.height() };
        (along(m.left, m.width(), a.left, a.width()), top)
    };
    (left - m.left, top - m.top)
}

pub fn attached(layout: &PairLayout, a: (Uuid, &DisplaySet), b: (Uuid, &DisplaySet)) -> PairLayout {
    let (anchor, moving) = if *a.0.as_bytes() == layout.input_host {
        (a, b)
    } else {
        (b, a)
    };
    let (Some(anchor_pos), Some(moving_pos)) = (
        layout.position(anchor.0.as_bytes()),
        layout.position(moving.0.as_bytes()),
    ) else {
        return layout.clone();
    };
    let (Some(anchor_area), Some(moving_area)) =
        (Area::of(anchor.1, anchor_pos), Area::of(moving.1, moving_pos))
    else {
        return layout.clone();
    };
    if touching(&moving_area, &anchor_area) {
        return layout.clone();
    }
    let (dx, dy) = snap(&moving_area, &anchor_area);
    let mut fixed = layout.clone();
    for p in fixed.positions.iter_mut() {
        if p.device_id == *moving.0.as_bytes() {
            p.x = (p.x as f64 + dx).round() as i32;
            p.y = (p.y as f64 + dy).round() as i32;
        }
    }
    fixed
}
