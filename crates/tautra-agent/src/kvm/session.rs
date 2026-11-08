use std::sync::Arc;
use std::time::{Duration, Instant};

use tautra_input::{
    cursor, display, guide, CursorHider, DeviceGeometry, DragCheck, Geometry, GuideOverlay,
    HookController, HookOptions, InjectError, Injector,
};
use tautra_ipc::api::{
    app_error, KvmRole, KvmState, LayoutDevice, LayoutMonitor, LayoutView, NoticeLevel,
};
use tautra_ipc::jsonrpc::RpcError;
use tautra_net::{write_msg, SessionHandle};
use tautra_proto::kvm::{
    decode_mouse_move, DevicePosition, DisplaySet, KvmMessage, MouseButton, PairLayout,
    StreamHeader, StreamKind,
};
use tautra_proto::share::FileOffer;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

use super::hook_options;
use super::layout::{self, DeviceSpec};
use super::sink::KvmSink;
use crate::app::Agent;

const DISPLAY_POLL: Duration = Duration::from_secs(2);
const ASSIST_REASSERT_TICKS: u32 = 5;
const BLOCKED_NOTICE_INTERVAL: Duration = Duration::from_secs(15);
const QUEUE: usize = 512;
const DRAG_POLL: Duration = Duration::from_millis(15);
const DRAG_PROBE_LIMIT: Duration = Duration::from_millis(400);
const DRAG_DROP_LIMIT: Duration = Duration::from_millis(1500);
const DRAG_WAIT: Duration = Duration::from_millis(1500);
const GUIDE_TTL: Duration = Duration::from_secs(6);
const KVM_DRAIN: Duration = Duration::from_millis(700);

pub enum SessionCmd {
    SetLayout {
        input_host: Uuid,
        positions: Vec<(Uuid, i32, i32)>,
        reply: oneshot::Sender<Result<(), RpcError>>,
    },
    Options(HookOptions),
    Locked(bool),
    Gaming(bool),
    Message(KvmMessage),
    Guide(Option<Vec<(Uuid, i32, i32)>>),
    Rebind(SessionHandle),
}

struct RemoteDrag {
    started: Instant,
    dropping: bool,
    x: i32,
    y: i32,
}

enum PendingKind {
    Offer(FileOffer),
    Return(u64),
}

struct PendingDrag {
    kind: PendingKind,
    until: tokio::time::Instant,
}

struct Peer {
    displays: DisplaySet,
    portable: bool,
}

struct Ctx {
    agent: Arc<Agent>,
    local_id: Uuid,
    peer_id: Uuid,
    peer_name: String,
    local_displays: DisplaySet,
    portable: bool,
    peer: Option<Peer>,
    layout: Option<PairLayout>,
    hook: Option<HookController>,
    hook_failed: bool,
    assist_on: bool,
    assist_ticks: u32,
    sink: Arc<KvmSink>,
    out: mpsc::Sender<KvmMessage>,
    injector: Injector,
    controlled: bool,
    hider: Option<CursorHider>,
    last_seq: Option<u32>,
    blocked_at: Option<Instant>,
    display_error: bool,
    drag: Option<RemoteDrag>,
    drag_next: Option<tokio::time::Instant>,
    drag_hold: bool,
    pending_drag: Option<PendingDrag>,
    pending_next: Option<tokio::time::Instant>,
    guide: Option<GuideOverlay>,
    guide_until: Option<Instant>,
    guide_failed: bool,
    prefs_sent: Option<(bool, u8)>,
}

async fn wait_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn open_kvm_stream(conn: &quinn::Connection) -> Option<quinn::SendStream> {
    let mut send = match conn.open_uni().await {
        Ok(send) => send,
        Err(e) => {
            tracing::debug!("stream KVM tidak bisa dibuka: {e}");
            return None;
        }
    };
    let header = StreamHeader {
        kind: StreamKind::Kvm,
    };
    if let Err(e) = write_msg(&mut send, &header).await {
        tracing::debug!("header stream KVM gagal dikirim: {e}");
        return None;
    }
    Some(send)
}

async fn write_loop(
    mut conn_rx: watch::Receiver<quinn::Connection>,
    mut rx: mpsc::Receiver<KvmMessage>,
) {
    loop {
        let conn = conn_rx.borrow_and_update().clone();
        let Some(mut send) = open_kvm_stream(&conn).await else {
            if conn_rx.changed().await.is_err() {
                return;
            }
            continue;
        };
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(msg) = msg else {
                        return;
                    };
                    if let Err(e) = write_msg(&mut send, &msg).await {
                        tracing::warn!("stream KVM gagal ditulis: {e}");
                        if conn_rx.changed().await.is_err() {
                            return;
                        }
                        break;
                    }
                }
                changed = conn_rx.changed() => {
                    let _ = send.finish();
                    let _ = tokio::time::timeout(KVM_DRAIN, send.stopped()).await;
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

pub async fn run(
    agent: Arc<Agent>,
    session: SessionHandle,
    mut cmds: mpsc::UnboundedReceiver<SessionCmd>,
    mut in_rx: mpsc::Receiver<KvmMessage>,
) {
    let Some(local_id) = agent.local_device_id() else {
        tracing::warn!("KVM tidak dimulai: identitas perangkat belum tersedia");
        return;
    };
    let mut conn = session.conn.clone();
    let mut session_id = session.id;
    let (conn_tx, conn_rx) = watch::channel(conn.clone());
    let (out_tx, out_rx) = mpsc::channel(QUEUE);
    let writer = tokio::spawn(write_loop(conn_rx.clone(), out_rx));

    let mut ctx = Ctx {
        sink: Arc::new(KvmSink::new(agent.clone(), conn_rx, out_tx.clone())),
        agent: agent.clone(),
        local_id,
        peer_id: session.device_id,
        peer_name: session.device_name.clone(),
        local_displays: DisplaySet::default(),
        portable: display::is_portable(),
        peer: None,
        layout: None,
        hook: None,
        hook_failed: false,
        assist_on: false,
        assist_ticks: 0,
        out: out_tx,
        injector: Injector::new(),
        controlled: false,
        hider: None,
        last_seq: None,
        blocked_at: None,
        display_error: false,
        drag: None,
        drag_next: None,
        drag_hold: false,
        pending_drag: None,
        pending_next: None,
        guide: None,
        guide_until: None,
        guide_failed: false,
        prefs_sent: None,
    };
    ctx.injector.set_fallback(crate::service::fallback());
    ctx.local_displays = ctx.read_displays().unwrap_or_default();
    ctx.send(KvmMessage::Hello {
        displays: ctx.local_displays.clone(),
        layout: agent.kvm.stored_layout(ctx.peer_id),
        portable: ctx.portable,
    });
    ctx.send_prefs(false);
    ctx.publish();
    tracing::info!(peer = %ctx.peer_id, "KVM menunggu data layar dari perangkat lain");

    let mut poll = tokio::time::interval(DISPLAY_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    poll.tick().await;
    let mut reading = true;

    loop {
        tokio::select! {
            msg = in_rx.recv(), if reading => match msg {
                Some(msg) => ctx.on_message(msg),
                None => reading = false,
            },
            datagram = conn.read_datagram() => match datagram {
                Ok(bytes) => ctx.on_datagram(&bytes),
                Err(_) => break,
            },
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::Rebind(next)) => {
                    tracing::info!(peer = %next.device_id, "KVM pindah ke koneksi baru tanpa berhenti");
                    conn = next.conn.clone();
                    session_id = next.id;
                    conn_tx.send_replace(conn.clone());
                }
                Some(cmd) => ctx.on_command(cmd),
                None => break,
            },
            _ = poll.tick() => ctx.poll_displays(),
            _ = wait_until(ctx.drag_next) => ctx.drag_step(),
            _ = wait_until(ctx.pending_next) => ctx.pending_step(),
            _ = agent.cancel.cancelled() => break,
        }
    }

    ctx.finish();
    writer.abort();
    agent.kvm.session_finished(session_id);
    agent.touch();
    tracing::info!(peer = %session.device_id, "KVM berhenti");
}

impl Ctx {
    fn send(&self, msg: KvmMessage) {
        if let Err(e) = self.out.try_send(msg) {
            tracing::warn!("pesan KVM tidak terkirim: {e}");
        }
    }

    fn read_displays(&mut self) -> Option<DisplaySet> {
        match display::enumerate() {
            Ok(displays) => {
                self.display_error = false;
                Some(displays)
            }
            Err(e) => {
                if !self.display_error {
                    self.display_error = true;
                    tracing::error!("{e}");
                    self.agent
                        .notice(NoticeLevel::Error, app_error::DISPLAY_FAILED, e.to_string());
                }
                None
            }
        }
    }

    fn is_host(&self) -> bool {
        self.layout
            .as_ref()
            .is_some_and(|l| l.input_host == *self.local_id.as_bytes())
    }

    fn valid(&self, layout: &PairLayout) -> bool {
        layout::is_valid(layout, self.local_id, self.peer_id)
    }

    fn default_layout(&self, revision: u64) -> Option<PairLayout> {
        let peer = self.peer.as_ref()?;
        Some(layout::default_layout(
            &DeviceSpec {
                id: self.local_id,
                displays: &self.local_displays,
                portable: self.portable,
            },
            &DeviceSpec {
                id: self.peer_id,
                displays: &peer.displays,
                portable: peer.portable,
            },
            revision,
        ))
    }

    fn save(&self, layout: &PairLayout) -> Result<(), String> {
        self.agent
            .kvm
            .save_layout(self.peer_id, layout)
            .inspect_err(|e| {
                tracing::error!("layout layar gagal disimpan: {e}");
                self.agent.notice(
                    NoticeLevel::Error,
                    app_error::LAYOUT_SAVE_FAILED,
                    format!("Layout layar gagal disimpan: {e}"),
                );
            })
    }

    fn merge_layout(&mut self, theirs: Option<PairLayout>) {
        let mine = self
            .agent
            .kvm
            .stored_layout(self.peer_id)
            .filter(|l| self.valid(l));
        let theirs_valid = theirs.clone().filter(|l| self.valid(l));
        let chosen = match (mine.clone(), theirs_valid) {
            (Some(a), Some(b)) => {
                if b.is_newer_than(&a) {
                    b
                } else {
                    a
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => match self.default_layout(0) {
                Some(l) => l,
                None => return,
            },
        };
        let chosen = if chosen.revision == 0 {
            self.default_layout(0).unwrap_or(chosen)
        } else {
            chosen
        };
        if mine.as_ref() != Some(&chosen) {
            let _ = self.save(&chosen);
        }
        if theirs.as_ref() != Some(&chosen) {
            self.send(KvmMessage::Layout(chosen.clone()));
        }
        tracing::info!(
            revision = chosen.revision,
            host = %Uuid::from_bytes(chosen.input_host),
            "layout layar disepakati"
        );
        self.layout = Some(chosen);
    }

    fn accept_layout(&mut self, layout: PairLayout) -> bool {
        if !self.valid(&layout) {
            tracing::warn!("layout dari perangkat lain tidak valid, diabaikan");
            return false;
        }
        if self
            .layout
            .as_ref()
            .is_some_and(|current| !layout.is_newer_than(current))
        {
            return false;
        }
        let _ = self.save(&layout);
        self.layout = Some(layout);
        true
    }

    fn effective_layout(&self) -> Option<PairLayout> {
        let layout = self.layout.as_ref()?;
        let peer = self.peer.as_ref()?;
        Some(layout::attached(
            layout,
            (self.local_id, &self.local_displays),
            (self.peer_id, &peer.displays),
        ))
    }

    fn geometry(&self) -> Option<Geometry> {
        let layout = &self.effective_layout()?;
        let peer = self.peer.as_ref()?;
        let local = layout.position(self.local_id.as_bytes())?;
        let remote = layout.position(self.peer_id.as_bytes())?;
        Some(Geometry {
            local: DeviceGeometry::new(&self.local_displays, local),
            remote: DeviceGeometry::new(&peer.displays, remote),
        })
    }

    fn refresh(&mut self) {
        if self.layout.as_ref().is_some_and(|l| l.revision == 0) {
            if let Some(fresh) = self.default_layout(0) {
                self.layout = Some(fresh);
            }
        }
        if self.layout.is_some() && self.peer.is_some() && self.is_host() {
            if self.controlled {
                self.controlled = false;
                self.injector.release_all();
            }
            self.hider = None;
            let geometry = self.geometry();
            match &self.hook {
                Some(hook) => hook.set_geometry(geometry),
                None => {
                    let (locked, gaming) = self.agent.kvm.toggles();
                    let options = hook_options(&self.agent.config().input);
                    match HookController::start(
                        self.sink.clone(),
                        options,
                        geometry,
                        locked,
                        gaming,
                    ) {
                        Ok(hook) => {
                            self.hook = Some(hook);
                            self.hook_failed = false;
                            self.set_assist(true);
                        }
                        Err(e) => {
                            if !self.hook_failed {
                                self.hook_failed = true;
                                tracing::error!("{e}");
                                self.agent.notice(
                                    NoticeLevel::Error,
                                    app_error::INPUT_HOOK_FAILED,
                                    e.to_string(),
                                );
                            }
                        }
                    }
                }
            }
        } else {
            self.hook = None;
            self.set_assist(false);
        }
        self.publish();
    }

    fn set_assist(&mut self, on: bool) {
        if self.assist_on == on {
            return;
        }
        self.assist_on = on;
        self.assist_ticks = 0;
        crate::service::set_assist(on);
    }

    fn reassert_assist(&mut self) {
        if !self.assist_on {
            return;
        }
        self.assist_ticks += 1;
        if self.assist_ticks >= ASSIST_REASSERT_TICKS {
            self.assist_ticks = 0;
            crate::service::set_assist(true);
        }
    }

    fn view(&self) -> Option<LayoutView> {
        let layout = &self.effective_layout()?;
        let peer = self.peer.as_ref()?;
        let device =
            |id: Uuid, name: String, is_local: bool, displays: &DisplaySet, portable: bool| {
                let scale = displays.scale();
                let (x, y) = layout.position(id.as_bytes()).unwrap_or((0, 0));
                LayoutDevice {
                    device_id: id.to_string(),
                    device_name: name,
                    is_local,
                    portable,
                    x,
                    y,
                    monitors: displays
                        .monitors
                        .iter()
                        .map(|m| LayoutMonitor {
                            name: m.name.clone(),
                            x: (m.x as f64 / scale).round() as i32,
                            y: (m.y as f64 / scale).round() as i32,
                            width: (m.width as f64 / scale).round() as i32,
                            height: (m.height as f64 / scale).round() as i32,
                            scale_percent: m.scale_percent,
                            diagonal_inch: m.physical_scale().map(|_| {
                                let (w, h) = (m.width_mm as f64, m.height_mm as f64);
                                ((w * w + h * h).sqrt() / 25.4).round() as u32
                            }),
                            primary: m.primary,
                        })
                        .collect(),
                }
            };
        Some(LayoutView {
            revision: layout.revision,
            input_host: Uuid::from_bytes(layout.input_host).to_string(),
            devices: vec![
                device(
                    self.local_id,
                    self.agent.config().effective_device_name(),
                    true,
                    &self.local_displays,
                    self.portable,
                ),
                device(
                    self.peer_id,
                    self.peer_name.clone(),
                    false,
                    &peer.displays,
                    peer.portable,
                ),
            ],
        })
    }

    fn publish(&self) {
        let ready = self.peer.is_some() && self.layout.is_some();
        let host = self.is_host();
        let view = self.view();
        let controlled = self.controlled;
        self.agent.kvm.update(|s| {
            s.state = if ready {
                KvmState::Ready
            } else {
                KvmState::Waiting
            };
            s.role = ready.then_some(if host {
                KvmRole::InputHost
            } else {
                KvmRole::Remote
            });
            s.controlled_by_peer = controlled;
            if !host {
                s.active = tautra_ipc::api::KvmActive::Local;
            }
            s.layout = view;
        });
        self.agent.touch();
    }

    fn injected(&mut self, result: Result<(), InjectError>) {
        if let Err(e) = result {
            if self
                .blocked_at
                .is_none_or(|t| t.elapsed() >= BLOCKED_NOTICE_INTERVAL)
            {
                self.blocked_at = Some(Instant::now());
                tracing::warn!("{e}");
                self.agent.notice(
                    NoticeLevel::Warning,
                    app_error::INPUT_INJECT_BLOCKED,
                    e.to_string(),
                );
            }
        }
    }

    fn on_message(&mut self, msg: KvmMessage) {
        match msg {
            KvmMessage::Hello {
                displays,
                layout,
                portable,
            } => {
                self.peer = Some(Peer { displays, portable });
                self.merge_layout(layout);
                self.refresh();
            }
            KvmMessage::Displays(displays) => {
                if let Some(peer) = &mut self.peer {
                    peer.displays = displays;
                }
                self.refresh();
            }
            KvmMessage::Layout(layout) => {
                if self.accept_layout(layout) {
                    self.refresh();
                }
            }
            KvmMessage::Enter { x, y, edge } => {
                if self.is_host() {
                    return;
                }
                self.controlled = true;
                self.drag_hold = false;
                tracing::info!(
                    x,
                    y,
                    cursor = ?tautra_input::cursor::position(),
                    hider = self.hider.is_some(),
                    ?edge,
                    "perangkat ini mulai dikendalikan"
                );
                self.hider = None;
                match tautra_input::cursor::restore() {
                    Ok(()) => tracing::info!("kursor perangkat ini dimunculkan saat diambil alih"),
                    Err(e) => tracing::warn!("kursor perangkat ini gagal dimunculkan saat diambil alih: {e}"),
                }
                let result = self.injector.move_to(x, y);
                self.injected(result);
                tracing::info!(cursor = ?tautra_input::cursor::position(), "kursor diletakkan di titik masuk");
                let input = self.agent.config().input;
                if input.crossing_animation {
                    tautra_input::crossing::play(
                        x,
                        y,
                        tautra_input::crossing::CrossKind::Enter,
                        edge.map(Into::into),
                        input.crossing_style,
                    );
                }
                self.publish();
            }
            KvmMessage::Leave { x, y, edge } => {
                tracing::info!(
                    controlled = self.controlled,
                    x,
                    y,
                    ?edge,
                    cursor = ?tautra_input::cursor::position(),
                    "kursor meninggalkan perangkat ini"
                );
                self.drag_hold = false;
                self.drop_pending("kursor sudah kembali ke perangkat lain");
                self.agent.share.cancel_drag();
                self.injector.release_all();
                if self.controlled {
                    self.controlled = false;
                    let input = self.agent.config().input;
                    if input.crossing_animation {
                        tautra_input::crossing::play(
                            x,
                            y,
                            tautra_input::crossing::CrossKind::Exit,
                            edge.map(Into::into),
                            input.crossing_style,
                        );
                    }
                    self.hide_idle_cursor();
                    self.publish();
                }
            }
            KvmMessage::Button { button, down, x, y } => {
                if !self.controlled {
                    return;
                }
                if button == MouseButton::Left && !down && self.drag_hold {
                    self.drag_hold = false;
                    if !self.agent.share.drag_active() {
                        self.drop_pending("tombol mouse dilepas sebelum drag siap");
                        return;
                    }
                }
                let result = self.injector.button(button, down, x, y);
                self.injected(result);
            }
            KvmMessage::Wheel { delta, horizontal } => {
                if self.controlled {
                    let result = self.injector.wheel(delta, horizontal);
                    self.injected(result);
                }
            }
            KvmMessage::Key {
                scan,
                vk,
                extended,
                down,
            } => {
                if self.controlled {
                    let result = self.injector.key(scan, vk, extended, down);
                    self.injected(result);
                }
            }
            KvmMessage::ReleaseAll => self.injector.release_all(),
            KvmMessage::DragProbe => self.start_drag_probe(),
            KvmMessage::DragResult { files } => {
                if let Some(hook) = &self.hook {
                    hook.remote_drag(files);
                }
            }
            KvmMessage::DragHold => {
                if self.controlled {
                    self.drag_hold = true;
                    self.pending_step();
                }
            }
            KvmMessage::DragFiles(offer) => self.queue_drag(PendingKind::Offer(offer)),
            KvmMessage::DragReturn { offer_id } => self.queue_drag(PendingKind::Return(offer_id)),
            KvmMessage::LayoutGuide(positions) => self.show_guide(positions.as_deref()),
            KvmMessage::Prefs {
                crossing_animation,
                crossing_style,
                changed,
            } => self.apply_peer_prefs(crossing_animation, crossing_style, changed),
        }
    }

    fn queue_drag(&mut self, kind: PendingKind) {
        if self.drag_ready() {
            self.begin_drag(kind);
            return;
        }
        self.drop_pending("digantikan drag yang lebih baru");
        tracing::info!("data drag tiba lebih dulu, menunggu tombol mouse");
        let now = tokio::time::Instant::now();
        self.pending_drag = Some(PendingDrag {
            kind,
            until: now + DRAG_WAIT,
        });
        self.pending_next = Some(now + DRAG_POLL);
    }

    fn begin_drag(&self, kind: PendingKind) {
        match kind {
            PendingKind::Offer(offer) => self.agent.share.receive_drag(&self.agent, offer),
            PendingKind::Return(offer_id) => self.agent.share.resume_drag(&self.agent, offer_id),
        }
    }

    fn drag_ready(&self) -> bool {
        match &self.hook {
            Some(hook) if self.is_host() => hook.drag_held(),
            _ => self.controlled && self.drag_hold,
        }
    }

    fn pending_step(&mut self) {
        let Some(pending) = self.pending_drag.as_ref() else {
            self.pending_next = None;
            return;
        };
        let now = tokio::time::Instant::now();
        if self.drag_ready() {
            self.pending_next = None;
            if let Some(pending) = self.pending_drag.take() {
                self.begin_drag(pending.kind);
            }
        } else if now >= pending.until {
            self.drop_pending("tombol mouse tidak ditahan lagi");
        } else {
            self.pending_next = Some(now + DRAG_POLL);
        }
    }

    fn drop_pending(&mut self, reason: &str) {
        self.pending_next = None;
        match self.pending_drag.take().map(|p| p.kind) {
            Some(PendingKind::Offer(offer)) => {
                tracing::info!(offer = offer.id, "drag dibatalkan: {reason}");
                self.agent.share.decline_drag(&self.agent, offer.id);
            }
            Some(PendingKind::Return(offer_id)) => {
                tracing::info!(offer = offer_id, "drag yang kembali dibatalkan: {reason}");
            }
            None => {}
        }
    }

    fn start_drag_probe(&mut self) {
        if self.controlled {
            if let Some(offer_id) = self.agent.share.take_incoming_drag() {
                self.send(KvmMessage::DragReturn { offer_id });
                self.send(KvmMessage::DragResult { files: true });
                return;
            }
        }
        let enabled = self.agent.config().share.drag_drop;
        let position = cursor::position();
        let probing = match position {
            Some((x, y)) if self.controlled && self.drag.is_none() => {
                self.agent.share.drag_probe(enabled, x, y)
            }
            _ => false,
        };
        match (probing, position) {
            (true, Some((x, y))) => {
                self.drag = Some(RemoteDrag {
                    started: Instant::now(),
                    dropping: false,
                    x,
                    y,
                });
                self.drag_next = Some(tokio::time::Instant::now() + DRAG_POLL);
            }
            _ => self.send(KvmMessage::DragResult { files: false }),
        }
    }

    fn finish_drag(&mut self, files: bool) {
        self.drag = None;
        self.drag_next = None;
        self.send(KvmMessage::DragResult { files });
    }

    fn drag_step(&mut self) {
        let Some(drag) = self.drag.as_ref() else {
            self.drag_next = None;
            return;
        };
        let (dropping, elapsed, x, y) = (drag.dropping, drag.started.elapsed(), drag.x, drag.y);
        let check = self.agent.share.drag_check();
        if !dropping {
            match check {
                DragCheck::Files => {
                    self.agent.share.drag_drop(false);
                    let result = self.injector.button(MouseButton::Left, false, x, y);
                    self.injected(result);
                    if let Some(drag) = self.drag.as_mut() {
                        drag.dropping = true;
                        drag.started = Instant::now();
                    }
                }
                DragCheck::Pending if elapsed < DRAG_PROBE_LIMIT => {}
                _ => {
                    self.agent.share.drag_cancel();
                    self.finish_drag(false);
                    return;
                }
            }
        } else {
            match check {
                DragCheck::Dropped => {
                    tracing::info!("drag file dikirim ke perangkat input host");
                    self.finish_drag(true);
                    return;
                }
                DragCheck::Failed => {
                    self.finish_drag(false);
                    return;
                }
                _ if elapsed > DRAG_DROP_LIMIT => {
                    self.agent.share.drag_cancel();
                    self.finish_drag(false);
                    return;
                }
                _ => {}
            }
        }
        self.drag_next = Some(tokio::time::Instant::now() + DRAG_POLL);
    }

    fn on_datagram(&mut self, bytes: &[u8]) {
        if !self.controlled {
            return;
        }
        let Some((seq, x, y)) = decode_mouse_move(bytes) else {
            return;
        };
        if self
            .last_seq
            .is_some_and(|last| (seq.wrapping_sub(last) as i32) <= 0)
        {
            return;
        }
        self.last_seq = Some(seq);
        if self.agent.share.drag_arming() {
            return;
        }
        let result = self.injector.move_to(x, y);
        self.injected(result);
    }

    fn set_layout(
        &mut self,
        input_host: Uuid,
        positions: Vec<(Uuid, i32, i32)>,
    ) -> Result<(), RpcError> {
        let Some(current) = self.layout.as_ref().filter(|_| self.peer.is_some()) else {
            return Err(RpcError::app(
                app_error::KVM_NOT_READY,
                "Data layar perangkat lain belum diterima",
            ));
        };
        let mut positions: Vec<DevicePosition> = positions
            .into_iter()
            .map(|(id, x, y)| DevicePosition {
                device_id: *id.as_bytes(),
                x,
                y,
            })
            .collect();
        positions.sort_by_key(|p| p.device_id);
        positions.dedup_by_key(|p| p.device_id);
        let layout = PairLayout {
            revision: current.revision + 1,
            input_host: *input_host.as_bytes(),
            positions,
        };
        if !self.valid(&layout) {
            return Err(RpcError::app(
                app_error::LAYOUT_INVALID,
                format!(
                    "Layout harus berisi posisi kedua perangkat dengan koordinat maksimal ±{}",
                    layout::MAX_COORDINATE
                ),
            ));
        }
        self.save(&layout)
            .map_err(|e| RpcError::app(app_error::LAYOUT_SAVE_FAILED, e))?;
        self.send(KvmMessage::Layout(layout.clone()));
        tracing::info!(revision = layout.revision, "layout layar diubah dari UI");
        self.layout = Some(layout);
        self.refresh();
        Ok(())
    }

    fn on_command(&mut self, cmd: SessionCmd) {
        match cmd {
            SessionCmd::SetLayout {
                input_host,
                positions,
                reply,
            } => {
                let result = self.set_layout(input_host, positions);
                let _ = reply.send(result);
            }
            SessionCmd::Options(options) => {
                if let Some(hook) = &self.hook {
                    hook.set_options(options);
                }
                if self.prefs_sent != Some((options.crossing_animation, options.crossing_style.code())) {
                    self.send_prefs(true);
                }
            }
            SessionCmd::Locked(on) => {
                if let Some(hook) = &self.hook {
                    hook.set_locked(on);
                }
            }
            SessionCmd::Gaming(on) => {
                if let Some(hook) = &self.hook {
                    hook.set_gaming(on);
                }
            }
            SessionCmd::Message(msg) => self.send(msg),
            SessionCmd::Rebind(_) => {}
            SessionCmd::Guide(positions) => {
                let positions = positions.map(|list| {
                    list.into_iter()
                        .map(|(id, x, y)| DevicePosition {
                            device_id: *id.as_bytes(),
                            x,
                            y,
                        })
                        .collect::<Vec<_>>()
                });
                self.show_guide(positions.as_deref());
                self.send(KvmMessage::LayoutGuide(positions));
            }
        }
    }

    fn show_guide(&mut self, positions: Option<&[DevicePosition]>) {
        let Some(positions) = positions else {
            self.hide_guide();
            return;
        };
        let find = |id: &Uuid| {
            positions
                .iter()
                .find(|p| &p.device_id == id.as_bytes())
                .map(|p| (p.x, p.y))
        };
        let (Some(peer), Some(local), Some(remote)) =
            (self.peer.as_ref(), find(&self.local_id), find(&self.peer_id))
        else {
            self.hide_guide();
            return;
        };
        let geometry = Geometry {
            local: DeviceGeometry::new(&self.local_displays, local),
            remote: DeviceGeometry::new(&peer.displays, remote),
        };
        let strips = guide::strips(&geometry);
        if self.guide.is_none() {
            match GuideOverlay::start() {
                Ok(overlay) => {
                    self.guide = Some(overlay);
                    self.guide_failed = false;
                }
                Err(e) => {
                    if !self.guide_failed {
                        self.guide_failed = true;
                        tracing::error!("{e}");
                        self.agent.notice(
                            NoticeLevel::Warning,
                            app_error::DISPLAY_FAILED,
                            format!("Panduan tepi layar tidak bisa ditampilkan: {e}"),
                        );
                    }
                    return;
                }
            }
        }
        if let Some(overlay) = &self.guide {
            overlay.show(strips);
        }
        self.guide_until = Some(Instant::now() + GUIDE_TTL);
    }

    fn hide_guide(&mut self) {
        self.guide = None;
        self.guide_until = None;
    }

    fn poll_displays(&mut self) {
        self.reassert_assist();
        if self.guide_until.is_some_and(|until| Instant::now() >= until) {
            self.hide_guide();
        }
        let Some(displays) = self.read_displays() else {
            return;
        };
        if displays != self.local_displays {
            tracing::info!("susunan monitor berubah");
            self.local_displays = displays.clone();
            self.send(KvmMessage::Displays(displays));
            self.refresh();
        } else if self.hook.is_none() && self.is_host() && self.peer.is_some() {
            self.refresh();
        }
    }

    fn send_prefs(&mut self, changed: bool) {
        let input = self.agent.config().input;
        let crossing_animation = input.crossing_animation;
        let crossing_style = input.crossing_style.code();
        self.prefs_sent = Some((crossing_animation, crossing_style));
        tracing::info!(
            crossing_animation,
            crossing_style,
            changed,
            "setelan bersama dikirim ke perangkat lain"
        );
        self.send(KvmMessage::Prefs {
            crossing_animation,
            crossing_style,
            changed,
        });
    }

    fn apply_peer_prefs(&mut self, crossing_animation: bool, crossing_style: u8, changed: bool) {
        let current = self.agent.config();
        let style = tautra_common::config::CrossingStyle::from_code(crossing_style);
        tracing::info!(
            crossing_animation,
            crossing_style,
            changed,
            local = current.input.crossing_animation,
            local_style = ?current.input.crossing_style,
            "setelan bersama diterima dari perangkat lain"
        );
        if style.is_none() {
            tracing::warn!(crossing_style, "gaya animasi dari perangkat lain tidak dikenal, diabaikan");
        }
        let style = style.unwrap_or(current.input.crossing_style);
        let same = current.input.crossing_animation == crossing_animation && current.input.crossing_style == style;
        if same || (!changed && !crossing_animation) {
            return;
        }
        let (crossing_animation, style) = if changed || !current.input.crossing_animation {
            (crossing_animation, style)
        } else {
            (current.input.crossing_animation, current.input.crossing_style)
        };
        if current.input.crossing_animation == crossing_animation && current.input.crossing_style == style {
            return;
        }
        self.prefs_sent = Some((crossing_animation, style.code()));
        let agent = self.agent.clone();
        tokio::spawn(async move {
            let mut next = agent.config();
            next.input.crossing_animation = crossing_animation;
            next.input.crossing_style = style;
            next.input.crossing_animation_asked = true;
            match agent.set_config(next).await {
                Ok(_) => tracing::info!(crossing_animation, ?style, "animasi perpindahan disamakan dengan perangkat lain"),
                Err(e) => tracing::warn!("setelan animasi dari perangkat lain gagal disimpan: {e}"),
            }
        });
    }

    fn hide_idle_cursor(&mut self) {
        match CursorHider::start() {
            Ok(hider) => {
                tracing::info!("kursor perangkat ini disembunyikan selama menganggur");
                self.hider = Some(hider)
            }
            Err(e) => tracing::warn!("kursor perangkat ini tidak bisa disembunyikan: {e}"),
        }
    }

    fn finish(&mut self) {
        if self.drag.take().is_some() {
            self.agent.share.drag_cancel();
        }
        self.agent.share.cancel_drag();
        self.drag_hold = false;
        self.drag_next = None;
        self.pending_drag = None;
        self.pending_next = None;
        self.hide_guide();
        self.hider = None;
        self.hook = None;
        self.set_assist(false);
        self.injector.release_all();
        self.controlled = false;
    }
}
