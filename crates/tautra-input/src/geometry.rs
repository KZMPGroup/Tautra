use tautra_proto::kvm::DisplaySet;

pub const ADJACENT_TOLERANCE: f64 = 2.0;
pub const ENTRY_INSET: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    Remote,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub(crate) fn horizontal(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectI {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RectI {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn center(&self) -> (i32, i32) {
        (
            self.left + (self.right - self.left) / 2,
            self.top + (self.bottom - self.top) / 2,
        )
    }

    pub fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x.clamp(self.left as f64, (self.right - 1).max(self.left) as f64),
            y.clamp(self.top as f64, (self.bottom - 1).max(self.top) as f64),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub phys: RectI,
    pub layout: RectF,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceGeometry {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub screens: Vec<Screen>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Located {
    Inside(usize),
    Outside(Edge),
}

impl DeviceGeometry {
    pub fn new(displays: &DisplaySet, offset: (i32, i32)) -> Self {
        let scale = displays.scale();
        let offset_x = offset.0 as f64;
        let offset_y = offset.1 as f64;
        let screens = displays
            .monitors
            .iter()
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| Screen {
                phys: RectI {
                    left: m.x,
                    top: m.y,
                    right: m.x + m.width,
                    bottom: m.y + m.height,
                },
                layout: RectF {
                    left: offset_x + m.x as f64 / scale,
                    top: offset_y + m.y as f64 / scale,
                    right: offset_x + (m.x + m.width) as f64 / scale,
                    bottom: offset_y + (m.y + m.height) as f64 / scale,
                },
                primary: m.primary,
            })
            .collect();
        Self {
            scale,
            offset_x,
            offset_y,
            screens,
        }
    }

    pub fn screen_at(&self, x: i32, y: i32) -> Option<usize> {
        self.screens.iter().position(|s| s.phys.contains(x, y))
    }

    pub fn primary_screen(&self) -> Option<usize> {
        self.screens
            .iter()
            .position(|s| s.primary)
            .or(if self.screens.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    pub fn to_layout(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.offset_x + x / self.scale,
            self.offset_y + y / self.scale,
        )
    }

    pub fn from_layout(&self, lx: f64, ly: f64) -> (f64, f64) {
        (
            (lx - self.offset_x) * self.scale,
            (ly - self.offset_y) * self.scale,
        )
    }

    pub fn exit_edge(&self, screen: usize, x: i32, y: i32) -> Option<Edge> {
        let r = self.screens.get(screen)?.phys;
        let cx = x.clamp(r.left, r.right - 1);
        let cy = y.clamp(r.top, r.bottom - 1);
        if x >= r.right - 1 && self.screen_at(r.right, cy).is_none() {
            Some(Edge::Right)
        } else if x <= r.left && self.screen_at(r.left - 1, cy).is_none() {
            Some(Edge::Left)
        } else if y >= r.bottom - 1 && self.screen_at(cx, r.bottom).is_none() {
            Some(Edge::Bottom)
        } else if y <= r.top && self.screen_at(cx, r.top - 1).is_none() {
            Some(Edge::Top)
        } else {
            None
        }
    }

    pub fn locate(&self, current: usize, x: f64, y: f64) -> Located {
        if let Some(i) = self.screen_at(x.floor() as i32, y.floor() as i32) {
            return Located::Inside(i);
        }
        let Some(r) = self.screens.get(current).map(|s| s.phys) else {
            return Located::Outside(Edge::Right);
        };
        let over_x = if x >= r.right as f64 {
            x - r.right as f64 + 1.0
        } else if x < r.left as f64 {
            r.left as f64 - x
        } else {
            0.0
        };
        let over_y = if y >= r.bottom as f64 {
            y - r.bottom as f64 + 1.0
        } else if y < r.top as f64 {
            r.top as f64 - y
        } else {
            0.0
        };
        if over_x >= over_y {
            Located::Outside(if x >= r.right as f64 {
                Edge::Right
            } else {
                Edge::Left
            })
        } else {
            Located::Outside(if y >= r.bottom as f64 {
                Edge::Bottom
            } else {
                Edge::Top
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub local: DeviceGeometry,
    pub remote: DeviceGeometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    pub screen: usize,
    pub x: i32,
    pub y: i32,
}

pub(crate) fn span(r: &RectF, horizontal_edge: bool) -> (f64, f64) {
    if horizontal_edge {
        (r.top, r.bottom)
    } else {
        (r.left, r.right)
    }
}

pub(crate) fn gap(src: &RectF, dst: &RectF, edge: Edge) -> f64 {
    match edge {
        Edge::Right => dst.left - src.right,
        Edge::Left => src.left - dst.right,
        Edge::Bottom => dst.top - src.bottom,
        Edge::Top => src.top - dst.bottom,
    }
}

impl Geometry {
    pub fn device(&self, side: Side) -> &DeviceGeometry {
        match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }

    pub fn speed_factor(&self) -> f64 {
        self.remote.scale / self.local.scale
    }

    pub fn is_usable(&self) -> bool {
        !self.local.screens.is_empty() && !self.remote.screens.is_empty()
    }

    pub fn cross(&self, from: Side, screen: usize, edge: Edge, x: f64, y: f64) -> Option<Crossing> {
        let src_dev = self.device(from);
        let dst_dev = self.device(from.other());
        let src = src_dev.screens.get(screen)?.layout;
        let horizontal = edge.horizontal();
        let (lx, ly) = src_dev.to_layout(x, y);
        let along = if horizontal { ly } else { lx };
        let (span_lo, span_hi) = span(&src, horizontal);

        let candidate = |r: &RectF| -> Option<f64> {
            let d = gap(&src, r, edge);
            if d < -ADJACENT_TOLERANCE {
                return None;
            }
            let (lo, hi) = span(r, horizontal);
            if hi.min(span_hi) - lo.max(span_lo) <= 0.0 {
                return None;
            }
            Some(d)
        };

        let nearest = dst_dev
            .screens
            .iter()
            .filter_map(|s| candidate(&s.layout))
            .fold(f64::INFINITY, f64::min);
        if !nearest.is_finite() {
            return None;
        }
        let in_group =
            |s: &Screen| candidate(&s.layout).is_some_and(|d| d <= nearest + ADJACENT_TOLERANCE);

        let along_c = along.clamp(span_lo, span_hi);
        let mut best: Option<(usize, f64)> = None;
        for (i, s) in dst_dev.screens.iter().enumerate() {
            if !in_group(s) {
                continue;
            }
            let (lo, hi) = span(&s.layout, horizontal);
            let lo = lo.max(span_lo);
            let hi = hi.min(span_hi);
            let dist = if along_c < lo {
                lo - along_c
            } else if along_c >= hi {
                along_c - hi
            } else {
                0.0
            };
            if dist > ADJACENT_TOLERANCE {
                continue;
            }
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }
        let (target, _) = best?;
        let (lo, hi) = span(&dst_dev.screens[target].layout, horizontal);
        let along_t = along_c.clamp(lo, hi);

        let r = dst_dev.screens[target].phys;
        let inset_x = ENTRY_INSET.min((r.right - r.left - 1).max(0));
        let inset_y = ENTRY_INSET.min((r.bottom - r.top - 1).max(0));
        let (px, py) = if horizontal {
            let (_, py) = dst_dev.from_layout(0.0, along_t);
            let x = match edge {
                Edge::Right => r.left + inset_x,
                _ => r.right - 1 - inset_x,
            };
            (x, py.round() as i32)
        } else {
            let (px, _) = dst_dev.from_layout(along_t, 0.0);
            let y = match edge {
                Edge::Bottom => r.top + inset_y,
                _ => r.bottom - 1 - inset_y,
            };
            (px.round() as i32, y)
        };
        Some(Crossing {
            screen: target,
            x: px.clamp(r.left, r.right - 1),
            y: py.clamp(r.top, r.bottom - 1),
        })
    }
}
