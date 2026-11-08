mod layout;
mod session;
mod sink;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tautra_common::config::InputConfig;
use tautra_common::hotkey::Hotkey;
use tautra_input::{ActiveSide, HookOptions, HookStatus, Hotkeys};
use tautra_ipc::api::{app_error, KvmActive, KvmStatus, LayoutGuideParams, LayoutSetParams};
use tautra_ipc::jsonrpc::RpcError;
use tautra_net::SessionHandle;
use tautra_proto::kvm::{KvmMessage, PairLayout};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::app::Agent;
use layout::LayoutStore;
use session::SessionCmd;

const KVM_QUEUE: usize = 512;

struct SessionLink {
    id: u64,
    peer: Uuid,
    tx: mpsc::UnboundedSender<SessionCmd>,
    messages: mpsc::Sender<KvmMessage>,
}

#[derive(Default)]
struct Shared {
    status: KvmStatus,
    link: Option<SessionLink>,
}

pub struct Kvm {
    shared: Mutex<Shared>,
    store: Mutex<LayoutStore>,
}

pub fn hook_options(config: &InputConfig) -> HookOptions {
    let parse = |text: &str| Hotkey::parse(text).ok();
    HookOptions {
        edge_delay: Duration::from_millis(config.edge_delay_ms as u64),
        hotkeys: Hotkeys {
            switch_local: parse(&config.hotkey_switch_local),
            switch_remote: parse(&config.hotkey_switch_remote),
            lock: parse(&config.hotkey_lock),
            gaming: parse(&config.hotkey_gaming),
        },
        gaming_auto: config.gaming_auto,
        crossing_animation: config.crossing_animation,
        crossing_style: config.crossing_style,
    }
}

impl Kvm {
    pub fn new() -> (Self, Option<String>) {
        let (store, warning) = LayoutStore::load();
        (
            Self {
                shared: Mutex::new(Shared::default()),
                store: Mutex::new(store),
            },
            warning,
        )
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().expect("kvm lock")
    }

    pub fn status(&self) -> KvmStatus {
        self.shared().status.clone()
    }

    fn update(&self, f: impl FnOnce(&mut KvmStatus)) {
        f(&mut self.shared().status);
    }

    fn toggles(&self) -> (bool, bool) {
        let s = self.shared();
        (s.status.locked, s.status.gaming)
    }

    fn hook_status(&self, status: HookStatus) {
        self.update(|s| {
            s.active = match status.active {
                ActiveSide::Local => KvmActive::Local,
                ActiveSide::Remote => KvmActive::Remote,
            };
            s.locked = status.locked;
            s.gaming = status.gaming;
        });
    }

    fn stored_layout(&self, peer: Uuid) -> Option<PairLayout> {
        self.store.lock().expect("layout lock").get(peer)
    }

    fn save_layout(&self, peer: Uuid, layout: &PairLayout) -> Result<(), String> {
        self.store.lock().expect("layout lock").put(peer, layout)
    }

    fn send(&self, cmd: SessionCmd) -> bool {
        self.shared()
            .link
            .as_ref()
            .is_some_and(|link| link.tx.send(cmd).is_ok())
    }

    pub fn session_started(
        &self,
        agent: Arc<Agent>,
        session: SessionHandle,
    ) -> mpsc::Sender<KvmMessage> {
        let mut shared = self.shared();
        if let Some(link) = shared.link.as_mut() {
            if link.peer == session.device_id
                && link.tx.send(SessionCmd::Rebind(session.clone())).is_ok()
            {
                link.id = session.id;
                return link.messages.clone();
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (messages, messages_rx) = mpsc::channel(KVM_QUEUE);
        shared.link = Some(SessionLink {
            id: session.id,
            peer: session.device_id,
            tx,
            messages: messages.clone(),
        });
        drop(shared);
        tokio::spawn(session::run(agent, session, rx, messages_rx));
        messages
    }

    fn session_finished(&self, id: u64) {
        let mut s = self.shared();
        if s.link.as_ref().is_some_and(|l| l.id == id) {
            s.link = None;
            s.status = KvmStatus {
                locked: s.status.locked,
                gaming: s.status.gaming,
                ..KvmStatus::default()
            };
        }
    }

    pub async fn set_layout(&self, params: LayoutSetParams) -> Result<(), RpcError> {
        let parse = |raw: &str| {
            Uuid::parse_str(raw.trim())
                .map_err(|_| RpcError::app(app_error::LAYOUT_INVALID, "device_id tidak valid"))
        };
        let input_host = parse(&params.input_host)?;
        let positions = params
            .positions
            .iter()
            .map(|p| Ok((parse(&p.device_id)?, p.x, p.y)))
            .collect::<Result<Vec<_>, RpcError>>()?;
        let (reply, rx) = oneshot::channel();
        let not_ready = || {
            RpcError::app(
                app_error::KVM_NOT_READY,
                "Layout layar hanya bisa diubah saat tersambung ke perangkat lain",
            )
        };
        if !self.send(SessionCmd::SetLayout {
            input_host,
            positions,
            reply,
        }) {
            return Err(not_ready());
        }
        rx.await.map_err(|_| not_ready())?
    }

    pub fn set_guide(&self, params: LayoutGuideParams) -> Result<(), RpcError> {
        let positions = match params.positions {
            Some(list) => Some(
                list.iter()
                    .map(|p| {
                        Uuid::parse_str(p.device_id.trim())
                            .map(|id| (id, p.x, p.y))
                            .map_err(|_| RpcError::app(app_error::LAYOUT_INVALID, "device_id tidak valid"))
                    })
                    .collect::<Result<Vec<_>, RpcError>>()?,
            ),
            None => None,
        };
        self.send(SessionCmd::Guide(positions));
        Ok(())
    }

    pub fn set_locked(&self, on: bool) {
        self.update(|s| s.locked = on);
        self.send(SessionCmd::Locked(on));
        tracing::info!(locked = on, "kunci kursor diubah dari UI");
    }

    pub fn set_gaming(&self, on: bool) {
        self.update(|s| {
            s.gaming = on;
            if on {
                s.active = KvmActive::Local;
            }
        });
        self.send(SessionCmd::Gaming(on));
        tracing::info!(gaming = on, "gaming mode diubah dari UI");
    }

    pub fn options_changed(&self, config: &InputConfig) {
        self.send(SessionCmd::Options(hook_options(config)));
    }

    pub fn send_message(&self, msg: KvmMessage) -> bool {
        self.send(SessionCmd::Message(msg))
    }
}
