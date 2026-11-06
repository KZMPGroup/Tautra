mod book;
mod clip;
mod net;
mod stage;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tautra_clipboard::{
    ClipboardHandler, ClipboardOptions, ClipboardService, RemoteContent, RemoteFiles,
};
use tautra_common::config::ShareConfig;
use tautra_common::time::unix_ms;
use tautra_input::DragCheck;
use tautra_ipc::api::{
    app_error, ClipActivity, NoticeLevel, ShareStatus, TransferDirection, TransferState,
};
use tautra_ipc::jsonrpc::RpcError;
use tautra_net::SessionHandle;
use tautra_proto::kvm::KvmMessage;
use tautra_proto::share::{ClipFormat, DropEffect, FileOffer, ShareMessage};
use tautra_transfer::{
    DragEffect, DragLabel, DragOutcome, DragRequest, DropProbe, Manifest, Prepare, ProbeState,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch, Notify};
use tokio_util::sync::CancellationToken;

use crate::app::Agent;
use book::{OfferBook, SharedFiles, SharedOffer, Transfer, Transfers};

pub use net::{serve_clip, serve_file};

const OUT_QUEUE: usize = 64;
const MAX_PENDING_DRAGS: usize = 8;
const TICK: Duration = Duration::from_millis(400);

struct PeerLink {
    session_id: u64,
    name: String,
    out: mpsc::Sender<ShareMessage>,
}

struct OutgoingDrag {
    offer_id: u64,
    sources: Vec<PathBuf>,
    name: String,
    dir: bool,
}

#[derive(Default)]
struct State {
    outgoing_drag: Option<OutgoingDrag>,
    incoming_drag: Option<u64>,
    peer: Option<PeerLink>,
    book: OfferBook,
    transfers: Transfers,
    pending: Vec<Transfer>,
    last_sent: Option<ClipActivity>,
    last_received: Option<ClipActivity>,
}

pub struct Share {
    state: Mutex<State>,
    conn: watch::Sender<Option<quinn::Connection>>,
    clipboard: Mutex<Option<ClipboardService>>,
    probe: Mutex<Option<DropProbe>>,
    active: Notify,
    runtime: Handle,
}

pub fn clipboard_options(config: &ShareConfig) -> ClipboardOptions {
    ClipboardOptions {
        enabled: config.clipboard,
        text: config.text,
        html: config.html,
        rtf: config.rtf,
        image: config.image,
        files: config.files,
        max_bytes: config.max_clipboard_bytes(),
    }
}

pub fn new_offer_id() -> u64 {
    uuid::Uuid::new_v4().as_u64_pair().0
}

impl Share {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            conn: watch::Sender::new(None),
            clipboard: Mutex::new(None),
            probe: Mutex::new(None),
            active: Notify::new(),
            runtime: Handle::current(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("share lock")
    }

    pub fn start(&self, agent: &Arc<Agent>) {
        let config = agent.config().share;
        let handler = Arc::new(clip::ClipHandler::new(agent, self.runtime.clone()));
        match ClipboardService::start(handler, clipboard_options(&config)) {
            Ok(service) => *self.clipboard.lock().expect("clipboard lock") = Some(service),
            Err(e) => {
                tracing::error!("{e}");
                agent.notice(NoticeLevel::Error, app_error::CLIPBOARD_FAILED, e.to_string());
            }
        }
        let weak = Arc::downgrade(agent);
        let runtime = self.runtime.clone();
        match DropProbe::start(move |paths| {
            if let Some(agent) = weak.upgrade() {
                runtime.spawn(offer_drag(agent, paths));
            }
        }) {
            Ok(probe) => *self.probe.lock().expect("probe lock") = Some(probe),
            Err(e) => {
                tracing::error!("{e}");
                agent.notice(NoticeLevel::Error, app_error::DRAGDROP_FAILED, e.to_string());
            }
        }
        tokio::spawn(ticker(agent.clone()));
    }

    pub fn shutdown(&self) {
        self.probe.lock().expect("probe lock").take();
        self.clipboard.lock().expect("clipboard lock").take();
        for transfer in self.state().transfers.iter_mut() {
            transfer.cancel.cancel();
        }
    }

    pub fn options_changed(&self, config: &ShareConfig) {
        if let Some(service) = self.clipboard.lock().expect("clipboard lock").as_ref() {
            service.set_options(clipboard_options(config));
        }
    }

    pub fn status(&self) -> ShareStatus {
        let clipboard_ready = self.clipboard.lock().expect("clipboard lock").is_some();
        let drag_ready = self.probe.lock().expect("probe lock").is_some();
        let s = self.state();
        ShareStatus {
            clipboard_ready,
            drag_ready,
            last_sent: s.last_sent.clone(),
            last_received: s.last_received.clone(),
            transfers: s.transfers.views(),
        }
    }

    pub fn connection(&self) -> Option<quinn::Connection> {
        self.conn
            .borrow()
            .clone()
            .filter(|c| c.close_reason().is_none())
    }

    pub fn connections(&self) -> watch::Receiver<Option<quinn::Connection>> {
        self.conn.subscribe()
    }

    fn connected(&self) -> bool {
        self.state().peer.is_some()
    }

    fn send(&self, msg: ShareMessage) -> bool {
        let out = self.state().peer.as_ref().map(|p| p.out.clone());
        match out {
            Some(out) => match out.try_send(msg) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("pesan clipboard/file tidak terkirim: {e}");
                    false
                }
            },
            None => false,
        }
    }

    pub fn session_started(&self, agent: &Arc<Agent>, session: &SessionHandle) {
        let (tx, rx) = mpsc::channel(OUT_QUEUE);
        {
            let mut s = self.state();
            s.peer = Some(PeerLink {
                session_id: session.id,
                name: session.device_name.clone(),
                out: tx,
            });
            for t in s.transfers.iter_mut() {
                if t.direction == TransferDirection::Outgoing && t.state == TransferState::Waiting {
                    t.state = TransferState::Running;
                }
            }
        }
        self.conn.send_replace(Some(session.conn.clone()));
        tokio::spawn(net::write_loop(session.conn.clone(), rx));
        let watcher = agent.clone();
        let conn = session.conn.clone();
        let id = session.id;
        tokio::spawn(async move {
            conn.closed().await;
            watcher.share.session_finished(id);
            watcher.touch();
        });
        agent.touch();
    }

    fn session_finished(&self, session_id: u64) {
        let mut s = self.state();
        if s.peer.as_ref().is_some_and(|p| p.session_id == session_id) {
            s.peer = None;
            for t in s.transfers.iter_mut() {
                if t.direction == TransferDirection::Outgoing && t.state == TransferState::Running {
                    t.state = TransferState::Waiting;
                }
            }
            drop(s);
            self.conn.send_replace(None);
        }
    }

    pub fn on_message(&self, agent: &Arc<Agent>, msg: ShareMessage) {
        match msg {
            ShareMessage::Clipboard(offer) => clip::receive(agent, offer),
            ShareMessage::DragEnd { offer_id, effect } => self.drag_ended(agent, offer_id, effect),
            ShareMessage::Revoke { offer_id } => {
                let mut s = self.state();
                if let Some(t) = s.transfers.by_offer(offer_id, TransferDirection::Incoming) {
                    if !t.finished() {
                        t.finish(
                            TransferState::Cancelled,
                            Some("dibatalkan oleh perangkat pengirim".into()),
                        );
                        t.cancel.cancel();
                    }
                }
                drop(s);
                agent.touch();
            }
            ShareMessage::Reject { offer_id, reason } => {
                let mut s = self.state();
                s.book.remove(offer_id);
                s.pending.retain(|t| t.offer_id != offer_id);
                if let Some(t) = s.transfers.by_offer(offer_id, TransferDirection::Outgoing) {
                    let state = if reason.contains("dibatalkan") {
                        TransferState::Cancelled
                    } else {
                        TransferState::Failed
                    };
                    t.finish(state, Some(reason));
                }
                drop(s);
                agent.touch();
            }
        }
    }

    pub(crate) fn set_remote_clipboard(&self, content: RemoteContent) -> bool {
        match self.clipboard.lock().expect("clipboard lock").as_ref() {
            Some(service) => {
                service.set_remote(content);
                true
            }
            None => false,
        }
    }

    fn record_sent(&self, activity: ClipActivity) {
        self.state().last_sent = Some(activity);
    }

    fn record_received(&self, activity: ClipActivity) {
        self.state().last_received = Some(activity);
    }

    fn offer_clipboard(
        &self,
        clip: Vec<(ClipFormat, Arc<Vec<u8>>)>,
        files: Option<Manifest>,
        message: impl FnOnce(u64) -> ShareMessage,
    ) -> bool {
        let id = new_offer_id();
        let completed = files.as_ref().map(|m| vec![false; m.entries.len()]).unwrap_or_default();
        self.state().book.insert(SharedOffer {
            id,
            push: None,
            created: Instant::now(),
            clip,
            files: files.map(|m| {
                Arc::new(SharedFiles {
                    entries: m.entries,
                    sources: m.sources,
                })
            }),
            sent: Arc::new(AtomicU64::new(0)),
            completed,
        });
        self.send(message(id))
    }

    fn start_drag_offer(&self, agent: &Arc<Agent>, manifest: Manifest) {
        let id = new_offer_id();
        let offer = FileOffer {
            id,
            entries: manifest.entries.clone(),
        };
        let peer_name = match self.state().peer.as_ref() {
            Some(peer) => peer.name.clone(),
            None => {
                agent.notice(
                    NoticeLevel::Error,
                    app_error::DRAGDROP_FAILED,
                    "File tidak bisa dikirim: perangkat lain tidak tersambung",
                );
                return;
            }
        };
        let names = offer.top_level();
        let sent = Arc::new(AtomicU64::new(0));
        let completed: Vec<bool> = manifest.entries.iter().map(|e| e.dir).collect();
        let roots: Vec<PathBuf> = manifest
            .entries
            .iter()
            .zip(manifest.sources.iter())
            .filter(|(entry, _)| !entry.path.contains('/'))
            .map(|(_, source)| source.clone())
            .collect();
        {
            let mut s = self.state();
            s.outgoing_drag = Some(OutgoingDrag {
                offer_id: id,
                sources: roots,
                name: names.first().map(|n| n.to_string()).unwrap_or_else(|| "file".into()),
                dir: manifest.entries.first().is_some_and(|e| e.dir),
            });
            let transfer_id = s.transfers.next_id();
            let transfer = Transfer {
                id: transfer_id,
                offer_id: id,
                direction: TransferDirection::Outgoing,
                peer_name,
                name: names.first().map(|n| n.to_string()).unwrap_or_default(),
                items: names.len() as u32,
                files: offer.file_count() as u32,
                total: offer.total_bytes(),
                base: sent.clone(),
                current: Arc::new(AtomicU64::new(0)),
                state: TransferState::Running,
                error: None,
                started_ms: unix_ms(),
                finished_ms: None,
                path: manifest.sources.first().cloned(),
                cancel: CancellationToken::new(),
            };
            s.pending.push(transfer);
            let excess = s.pending.len().saturating_sub(MAX_PENDING_DRAGS);
            s.pending.drain(..excess);
            s.book.insert(SharedOffer {
                id,
                push: Some(transfer_id),
                created: Instant::now(),
                clip: Vec::new(),
                files: Some(Arc::new(SharedFiles {
                    entries: manifest.entries,
                    sources: manifest.sources,
                })),
                sent,
                completed,
            });
        }
        tracing::info!(offer = id, items = names.len(), "file ditawarkan lewat drag-drop");
        if !agent.kvm.send_message(KvmMessage::DragFiles(offer)) {
            self.offer_failed(id, "Daftar file gagal dikirim ke perangkat lain");
            agent.notice(
                NoticeLevel::Error,
                app_error::DRAGDROP_FAILED,
                "Daftar file gagal dikirim ke perangkat lain",
            );
        }
        agent.touch();
    }

    fn offer_failed(&self, offer_id: u64, reason: &str) {
        let mut s = self.state();
        s.book.remove(offer_id);
        s.pending.retain(|t| t.offer_id != offer_id);
        tracing::warn!(offer = offer_id, "tawaran drag dibatalkan: {reason}");
    }

    fn activate_drag(&self, offer_id: u64) {
        let mut s = self.state();
        let Some(i) = s.pending.iter().position(|t| t.offer_id == offer_id) else {
            return;
        };
        let mut transfer = s.pending.remove(i);
        transfer.started_ms = unix_ms();
        if s.book.all_completed(offer_id) {
            transfer.finish(TransferState::Done, None);
        }
        s.transfers.add(transfer);
        drop(s);
        self.active.notify_one();
    }

    pub fn receive_drag(&self, agent: &Arc<Agent>, offer: FileOffer) {
        let config = agent.config().share;
        if !config.drag_drop {
            let reason = "File dari perangkat lain ditolak karena drag-drop dimatikan di perangkat ini";
            agent.notice(NoticeLevel::Warning, app_error::TRANSFER_FAILED, reason);
            self.send(ShareMessage::Reject {
                offer_id: offer.id,
                reason: reason.into(),
            });
            return;
        }
        let probe = self.probe();
        let Some(probe) = probe.as_ref() else {
            drop(probe);
            agent.notice(
                NoticeLevel::Warning,
                app_error::DRAGDROP_FAILED,
                "Drag dari perangkat lain dibatalkan karena drag-drop belum siap di perangkat ini",
            );
            self.decline_drag(agent, offer.id);
            return;
        };
        let handler: Arc<dyn ClipboardHandler> =
            Arc::new(clip::ClipHandler::new(agent, self.runtime.clone()));
        let content = RemoteContent {
            offer_id: offer.id,
            items: Vec::new(),
            files: Some(RemoteFiles {
                offer_id: offer.id,
                entries: offer.entries.clone(),
            }),
        };
        let label = DragLabel {
            name: offer
                .top_level()
                .first()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "file".into()),
            dir: offer.entries.first().is_some_and(|e| e.dir),
        };
        let weak = Arc::downgrade(agent);
        let runtime = self.runtime.clone();
        let stage = stage::DropStage::new(agent, self.runtime.clone(), offer.id, &offer.entries);
        let prepare = stage.clone().map(|stage| {
            let prepare: Prepare = Arc::new(move || stage.fill());
            prepare
        });
        tracing::info!(offer = offer.id, "drag file dari perangkat lain dimulai di sini");
        self.state().incoming_drag = Some(offer.id);
        probe.start_drag(DragRequest {
            data: Box::new(move || {
                Ok(match stage {
                    Some(stage) => tautra_clipboard::create_drag_object(content, handler, stage),
                    None => tautra_clipboard::create_data_object(content, handler),
                })
            }),
            label,
            native: false,
            prepare,
            done: Box::new(move |outcome| {
                if let Some(agent) = weak.upgrade() {
                    runtime.spawn(async move { agent.share.drag_finished(&agent, offer, outcome) });
                }
            }),
        });
    }

    pub fn decline_drag(&self, agent: &Arc<Agent>, offer_id: u64) {
        self.send(ShareMessage::DragEnd {
            offer_id,
            effect: DropEffect::None,
        });
        agent.touch();
    }

    pub fn take_incoming_drag(&self) -> Option<u64> {
        let active = self.drag_active();
        let offer_id = {
            let mut s = self.state();
            let taken = s.incoming_drag.take();
            taken.filter(|_| active)?
        };
        self.cancel_drag();
        tracing::info!(offer = offer_id, "drag dikembalikan ke perangkat asal");
        Some(offer_id)
    }

    pub fn resume_drag(&self, agent: &Arc<Agent>, offer_id: u64) {
        let found = self
            .state()
            .outgoing_drag
            .as_ref()
            .filter(|d| d.offer_id == offer_id)
            .map(|d| (d.sources.clone(), d.name.clone(), d.dir));
        let Some((sources, name, dir)) = found else {
            tracing::warn!(offer = offer_id, "drag yang kembali tidak dikenal, diabaikan");
            return;
        };
        let probe = self.probe();
        let Some(probe) = probe.as_ref() else {
            return;
        };
        let weak = Arc::downgrade(agent);
        tracing::info!(offer = offer_id, "drag kembali ke perangkat ini, dilanjutkan");
        probe.start_drag(DragRequest {
            data: Box::new(move || tautra_transfer::shell_data(&sources)),
            label: DragLabel { name, dir },
            native: true,
            prepare: None,
            done: Box::new(move |outcome| {
                if let (DragOutcome::Failed(reason), Some(agent)) = (outcome, weak.upgrade()) {
                    agent.notice(
                        NoticeLevel::Warning,
                        app_error::DRAGDROP_FAILED,
                        format!("Drag tidak bisa dilanjutkan di perangkat ini ({reason})"),
                    );
                }
            }),
        });
    }

    fn drag_finished(&self, agent: &Arc<Agent>, offer: FileOffer, outcome: DragOutcome) {
        {
            let mut s = self.state();
            if s.incoming_drag == Some(offer.id) {
                s.incoming_drag = None;
            }
        }
        match outcome {
            DragOutcome::Dropped(effect) => {
                let effect = match effect {
                    DragEffect::None => DropEffect::None,
                    DragEffect::Copy => DropEffect::Copy,
                };
                self.send(ShareMessage::DragEnd {
                    offer_id: offer.id,
                    effect,
                });
            }
            DragOutcome::Failed(reason) => {
                agent.notice(
                    NoticeLevel::Warning,
                    app_error::DRAGDROP_FAILED,
                    format!("Drag dibatalkan karena tidak bisa dimulai di perangkat ini ({reason})"),
                );
                self.send(ShareMessage::DragEnd {
                    offer_id: offer.id,
                    effect: DropEffect::None,
                });
            }
        }
        agent.touch();
    }

    fn drag_ended(&self, agent: &Arc<Agent>, offer_id: u64, effect: DropEffect) {
        tracing::info!(offer = offer_id, ?effect, "drag selesai di perangkat lain");
        match effect {
            DropEffect::None => {
                let mut s = self.state();
                s.book.remove(offer_id);
                s.pending.retain(|t| t.offer_id != offer_id);
            }
            DropEffect::Copy => self.activate_drag(offer_id),
        }
        agent.touch();
    }

    pub fn drag_active(&self) -> bool {
        self.probe().as_ref().is_some_and(DropProbe::dragging)
    }

    pub fn drag_arming(&self) -> bool {
        self.probe().as_ref().is_some_and(DropProbe::arming)
    }

    pub fn cancel_drag(&self) {
        if let Some(probe) = self.probe().as_ref() {
            probe.cancel_drag();
        }
    }

    pub(crate) fn reject_offer(&self, offer_id: u64, reason: String) {
        self.send(ShareMessage::Reject { offer_id, reason });
    }

    fn clip_data(&self, offer_id: u64, format: ClipFormat) -> Option<Arc<Vec<u8>>> {
        self.state().book.clip(offer_id, format)
    }

    fn file_lookup(&self, offer_id: u64, index: u32) -> Option<book::FileLookup> {
        self.state().book.file(offer_id, index)
    }

    fn file_served(&self, offer_id: u64, index: u32) {
        let mut s = self.state();
        if let Some((transfer, all_done)) = s.book.complete(offer_id, index) {
            if all_done {
                if let Some(t) = s.transfers.get_mut(transfer) {
                    t.finish(TransferState::Done, None);
                }
            }
        }
    }

    fn has_active(&self) -> bool {
        self.state().transfers.active()
    }

    pub fn cancel_transfer(&self, id: u64) -> Result<(), RpcError> {
        let mut s = self.state();
        let Some(t) = s.transfers.get_mut(id) else {
            return Err(not_found());
        };
        if t.finished() {
            return Ok(());
        }
        let offer_id = t.offer_id;
        let direction = t.direction;
        t.finish(TransferState::Cancelled, Some("dibatalkan".into()));
        t.cancel.cancel();
        if direction == TransferDirection::Outgoing {
            s.book.remove(offer_id);
        }
        drop(s);
        if direction == TransferDirection::Outgoing {
            self.send(ShareMessage::Revoke { offer_id });
        } else {
            self.reject_offer(offer_id, "dibatalkan oleh penerima".into());
        }
        tracing::info!(transfer = id, "transfer dibatalkan dari UI");
        Ok(())
    }

    pub fn reveal_transfer(&self, id: u64) -> Result<(), RpcError> {
        let path = self
            .state()
            .transfers
            .get(id)
            .ok_or_else(not_found)?
            .path
            .clone();
        let Some(path) = path.filter(|p| p.exists()) else {
            return Err(RpcError::app(
                app_error::TRANSFER_NOT_FOUND,
                "File hasil transfer tidak ditemukan lagi",
            ));
        };
        tautra_transfer::reveal(&path)
            .map_err(|e| RpcError::app(app_error::TRANSFER_FAILED, format!("Explorer gagal dibuka: {e}")))
    }

    pub fn clear_transfers(&self) {
        self.state().transfers.clear_finished();
    }

    fn probe(&self) -> MutexGuard<'_, Option<DropProbe>> {
        self.probe.lock().expect("probe lock")
    }

    pub fn drag_probe(&self, enabled: bool, x: i32, y: i32) -> bool {
        if !enabled || !self.connected() {
            return false;
        }
        match self.probe().as_ref() {
            Some(probe) => {
                probe.probe(x, y);
                true
            }
            None => false,
        }
    }

    pub fn drag_check(&self) -> DragCheck {
        match self.probe().as_ref().map(DropProbe::state) {
            Some(ProbeState::Probing) | Some(ProbeState::Dropping) => DragCheck::Pending,
            Some(ProbeState::Files) => DragCheck::Files,
            Some(ProbeState::Dropped) => DragCheck::Dropped,
            Some(ProbeState::Failed) => DragCheck::Failed,
            Some(ProbeState::NoFiles) | Some(ProbeState::Idle) | None => DragCheck::NoFiles,
        }
    }

    pub fn drag_drop(&self, inject_release: bool) {
        if let Some(probe) = self.probe().as_ref() {
            probe.drop_now(inject_release);
        }
    }

    pub fn drag_cancel(&self) {
        if let Some(probe) = self.probe().as_ref() {
            probe.cancel();
        }
    }
}

fn not_found() -> RpcError {
    RpcError::app(app_error::TRANSFER_NOT_FOUND, "Transfer tidak ditemukan")
}

async fn offer_drag(agent: Arc<Agent>, paths: Vec<PathBuf>) {
    let built = tokio::task::spawn_blocking(move || tautra_transfer::build_manifest(&paths)).await;
    match built {
        Ok(Ok(manifest)) => agent.share.start_drag_offer(&agent, manifest),
        Ok(Err(e)) => {
            tracing::error!("file drag-drop tidak bisa dibaca: {e}");
            agent.notice(
                NoticeLevel::Error,
                app_error::DRAGDROP_FAILED,
                format!("File yang diseret tidak bisa dikirim: {e}"),
            );
        }
        Err(e) => tracing::error!("pembacaan daftar file berhenti: {e}"),
    }
}

async fn ticker(agent: Arc<Agent>) {
    loop {
        if agent.share.has_active() {
            agent.touch();
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = agent.cancel.cancelled() => return,
            }
        } else {
            tokio::select! {
                _ = agent.share.active.notified() => {}
                _ = agent.cancel.cancelled() => return,
            }
            agent.touch();
        }
    }
}
