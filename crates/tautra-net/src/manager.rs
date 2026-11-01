use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use quinn::Endpoint;
use tautra_common::build;
use tautra_common::config::NetworkConfig;
use tautra_common::paths;
use tautra_common::time::unix_ms;
use tautra_ipc::api::{
    app_error, ConnectedPeer, ConnectionStatus, DiscoveredDevice, IdleReason, LocalDevice,
    NetworkInfo, NetworkStatus, NoticeLevel, PairingHostResult, PairingStatus,
    TrustedDevice,
};
use tautra_proto::control::Control;
use tautra_proto::pairing::PairingRejectReason;
use tautra_proto::{close_code, ALPN_PAIRING, ALPN_PROBE, ALPN_SESSION, ALPN_UPDATE, PROTOCOL_VERSION};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::discovery::{Discovery, DiscoveryEvent, LocalAdvert, PeerAdvert, TaggedEvent};
use crate::error::NetError;
use crate::fingerprint::Fingerprint;
use crate::identity::DeviceIdentity;
use crate::netwatch::{self, ConnectedNetwork, LocalAddr, NetWatcher};
use crate::pairing::{self, PairedPeer, PairingFailure};
use crate::knownnet::KnownNetworks;
use crate::pathmem::PathMemory;
use crate::qos::Qos;
use crate::session::{
    self, EndReason, Established, HandshakeError, LocalInfo, SessionEvent, SessionHandle,
    HANDSHAKE_TIMEOUT,
};
use crate::tls::{self, TlsContext, SERVER_NAME};
use crate::trust::{self, TrustStore, TrustedEntry};
use crate::update;

const PAIRING_WINDOW: Duration = Duration::from_secs(120);
const PAIRING_ATTEMPTS: u32 = 3;
const PAIRING_RESULT_VISIBLE: Duration = Duration::from_secs(15);
const PAIRING_JOIN_TIMEOUT: Duration = Duration::from_secs(15);
const PAIRING_BROWSE_WINDOW: Duration = Duration::from_secs(120);
const BACKOFF_INITIAL: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(10);
const NON_PREFERRED_DIAL_DELAY: Duration = Duration::from_millis(1500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IDLE_REQUERY: Duration = Duration::from_secs(30);
const NET_CHANGE_DEBOUNCE: Duration = Duration::from_millis(300);
const NLM_RECHECK_MS: [u64; 7] = [300, 1_000, 2_000, 3_000, 5_000, 8_000, 12_000];
const NETWORK_POLL_FALLBACK: Duration = Duration::from_secs(5);
const DISCOVERY_RETRY: Duration = Duration::from_secs(5);
const NOTICE_THROTTLE: Duration = Duration::from_secs(600);
const PROBE_SETTLE: Duration = Duration::from_secs(2);
const HANDOVER_GRACE: Duration = Duration::from_millis(1500);
const PROBE_ROUNDS: usize = 5;
const PROBE_HOLD: Duration = Duration::from_secs(3);
const WIRELESS_PENALTY: Duration = Duration::from_micros(500);
const SWITCH_MIN_GAIN: Duration = Duration::from_micros(200);
const MANUAL_VERSION_RETRY: Duration = Duration::from_secs(15);
const UPDATE_RETRY_MIN: Duration = Duration::from_secs(10);
const UPDATE_RETRY_MAX: Duration = Duration::from_secs(300);

pub struct NetOptions {
    pub network: NetworkConfig,
    pub device_name: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetStatus {
    pub local: LocalDevice,
    pub connection: ConnectionStatus,
    pub network: NetworkStatus,
    pub pairing: PairingStatus,
    pub discovered: Vec<DiscoveredDevice>,
    pub trusted: Vec<TrustedDevice>,
}

#[derive(Debug, Clone)]
pub enum NetEvent {
    SessionStarted(SessionHandle),
    SessionEnded {
        device_id: Uuid,
        device_name: String,
    },
    Notice {
        level: NoticeLevel,
        code: &'static str,
        message: String,
    },
    UpdateReady {
        device_name: String,
        build: u64,
        dir: std::path::PathBuf,
        files: Vec<std::path::PathBuf>,
    },
}

pub enum NetCommand {
    HostPairing {
        reply: oneshot::Sender<Result<PairingHostResult, NetError>>,
    },
    JoinPairing {
        device_id: Uuid,
        pin: String,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    JoinPairingAt {
        addrs: Vec<SocketAddr>,
        label: String,
        pin: String,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    BrowseForPairing,
    CancelPairing,
    RemoveDevice {
        device_id: Uuid,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    Disconnect,
    Resume,
    ForgetNetwork {
        name: String,
    },
    UpdateConfig {
        network: NetworkConfig,
        device_name: String,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct NetHandle {
    cmd_tx: mpsc::Sender<NetCommand>,
    status_rx: watch::Receiver<NetStatus>,
    events_tx: broadcast::Sender<NetEvent>,
    trust_path: std::path::PathBuf,
}

impl NetHandle {
    pub fn status(&self) -> NetStatus {
        self.status_rx.borrow().clone()
    }

    pub fn watch_status(&self) -> watch::Receiver<NetStatus> {
        self.status_rx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NetEvent> {
        self.events_tx.subscribe()
    }

    pub fn trust_store_path(&self) -> &std::path::Path {
        &self.trust_path
    }

    async fn request<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, NetError>>) -> NetCommand,
    ) -> Result<T, NetError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(make(tx))
            .await
            .map_err(|_| NetError::internal("manager jaringan tidak berjalan"))?;
        rx.await
            .map_err(|_| NetError::internal("manager jaringan berhenti"))?
    }

    async fn send(&self, cmd: NetCommand) {
        let _ = self.cmd_tx.send(cmd).await;
    }

    pub async fn host_pairing(&self) -> Result<PairingHostResult, NetError> {
        self.request(|reply| NetCommand::HostPairing { reply })
            .await
    }

    pub async fn join_pairing(&self, device_id: Uuid, pin: String) -> Result<(), NetError> {
        self.request(|reply| NetCommand::JoinPairing {
            device_id,
            pin,
            reply,
        })
        .await
    }

    pub async fn join_pairing_at(
        &self,
        addrs: Vec<SocketAddr>,
        label: String,
        pin: String,
    ) -> Result<(), NetError> {
        self.request(|reply| NetCommand::JoinPairingAt {
            addrs,
            label,
            pin,
            reply,
        })
        .await
    }

    pub async fn browse_for_pairing(&self) {
        self.send(NetCommand::BrowseForPairing).await
    }

    pub async fn cancel_pairing(&self) {
        self.send(NetCommand::CancelPairing).await
    }

    pub async fn remove_device(&self, device_id: Uuid) -> Result<(), NetError> {
        self.request(|reply| NetCommand::RemoveDevice { device_id, reply })
            .await
    }

    pub async fn disconnect(&self) {
        self.send(NetCommand::Disconnect).await
    }

    pub async fn resume(&self) {
        self.send(NetCommand::Resume).await
    }

    pub async fn forget_network(&self, name: String) {
        self.send(NetCommand::ForgetNetwork { name }).await
    }

    pub async fn update_config(&self, network: NetworkConfig, device_name: String) {
        self.send(NetCommand::UpdateConfig {
            network,
            device_name,
        })
        .await
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(NetCommand::Shutdown { reply: tx })
            .await
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(3), rx).await;
        }
    }
}

pub async fn start(
    opts: NetOptions,
) -> Result<(NetHandle, broadcast::Receiver<NetEvent>), NetError> {
    let identity_dir = paths::identity_dir()
        .map_err(|e| NetError::new(app_error::IDENTITY_FAILED, e.to_string()))?;
    let (identity, identity_warning) = DeviceIdentity::load_or_create(&identity_dir)
        .map_err(|e| NetError::new(app_error::IDENTITY_FAILED, e.to_string()))?;
    let trust_path = trust::default_path()
        .map_err(|e| NetError::new(app_error::TRUST_STORE_FAILED, e.to_string()))?;
    let (trust, trust_warning) = TrustStore::load(&trust_path);

    let tls = Arc::new(TlsContext::new(&identity));
    let server_config = tls
        .server_config()
        .map_err(|e| NetError::internal(e.to_string()))?;
    let (endpoint, qos, port, port_warning) = bind_endpoint(server_config, opts.network.port)?;

    let (events_tx, events_rx) = broadcast::channel(128);
    for (code, warning) in [
        (app_error::IDENTITY_FAILED, identity_warning),
        (app_error::TRUST_STORE_FAILED, trust_warning),
        (app_error::NETWORK_PORT_FALLBACK, port_warning),
    ] {
        if let Some(message) = warning {
            let _ = events_tx.send(NetEvent::Notice {
                level: NoticeLevel::Warning,
                code,
                message,
            });
        }
    }

    let local = LocalInfo {
        device_id: identity.device_id,
        device_name: opts.device_name.clone(),
        app_version: opts.app_version.clone(),
    };
    let (local_tx, local_rx) = watch::channel(local.clone());
    let trust = Arc::new(RwLock::new(trust));
    let (internal_tx, internal_rx) = mpsc::channel(256);
    let (session_tx, session_rx) = mpsc::channel(256);
    let (disc_tx, disc_rx) = mpsc::channel(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(32);

    let net_notify = Arc::new(Notify::new());
    let watcher = match NetWatcher::start(net_notify.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!("pemantau jaringan gagal dipasang: {e}");
            let _ = events_tx.send(NetEvent::Notice {
                level: NoticeLevel::Warning,
                code: app_error::INTERNAL,
                message: format!(
                    "Pemantau perubahan jaringan tidak aktif ({e}); jaringan dicek ulang setiap 5 detik."
                ),
            });
            None
        }
    };

    let (paths_tx, paths_rx) = watch::channel::<Option<Vec<u128>>>(
        opts.network.restrict_networks.then(Vec::new),
    );
    tokio::spawn(accept_loop(
        endpoint.clone(),
        local_rx,
        trust.clone(),
        internal_tx.clone(),
        paths_rx,
    ));

    let mut manager = Manager {
        local,
        local_tx,
        local_fp: identity.fingerprint,
        tls,
        endpoint,
        port,
        config: opts.network,
        trust,
        discovery: None,
        discovery_gen: 0,
        discovery_retry_at: None,
        discovery_failed: false,
        last_requery: Instant::now(),
        disc_tx,
        peers: HashMap::new(),
        fullnames: HashMap::new(),
        session: None,
        next_session_id: 1,
        dial: None,
        next_attempt_id: 1,
        paused: false,
        pairing: PairingState::Inactive,
        browse_until: None,
        networks: Vec::new(),
        networks_checked: false,
        network_allowed: false,
        known_networks: KnownNetworks::load(),
        qos,
        connected: Vec::new(),
        paths_tx,
        nlm_failed: false,
        local_ipv4: Vec::new(),
        memory: PathMemory::load(),
        nlm_checks: VecDeque::new(),
        net_change_at: None,
        poll_fallback: watcher.is_none(),
        last_poll: Instant::now(),
        path: PathSwitch::default(),
        update: UpdateState::default(),
        notice_log: HashMap::new(),
        status_tx: None,
        events_tx: events_tx.clone(),
        internal_tx,
        session_tx,
    };
    manager.local_ipv4 = netwatch::local_ipv4();
    manager.schedule_nlm_checks(Instant::now(), &[0]);
    let (status_tx, status_rx) = watch::channel(manager.build_status());
    manager.status_tx = Some(status_tx);

    tracing::info!(
        device_id = %manager.local.device_id,
        port,
        fingerprint = %identity.fingerprint.display(),
        "jaringan siap"
    );

    tokio::spawn(manager.run(
        cmd_rx,
        internal_rx,
        session_rx,
        disc_rx,
        net_notify,
        watcher,
    ));

    Ok((
        NetHandle {
            cmd_tx,
            status_rx,
            events_tx,
            trust_path,
        },
        events_rx,
    ))
}

fn open_endpoint(
    server_config: &quinn::ServerConfig,
    port: u16,
) -> std::io::Result<(Endpoint, Option<Qos>)> {
    let socket = std::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port)))?;
    let qos = Qos::new(&socket);
    let runtime = quinn::default_runtime()
        .ok_or_else(|| std::io::Error::other("runtime async tidak ditemukan"))?;
    let endpoint = Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_config.clone()),
        socket,
        runtime,
    )?;
    Ok((endpoint, qos))
}

type Bound = (Endpoint, Option<Qos>, u16, Option<String>);

fn bind_endpoint(server_config: quinn::ServerConfig, port: u16) -> Result<Bound, NetError> {
    let actual_port = |ep: &Endpoint| ep.local_addr().map(|a| a.port()).unwrap_or(0);
    match open_endpoint(&server_config, port) {
        Ok((ep, qos)) => {
            let p = actual_port(&ep);
            Ok((ep, qos, p, None))
        }
        Err(first) if port != 0 => match open_endpoint(&server_config, 0) {
            Ok((ep, qos)) => {
                let p = actual_port(&ep);
                tracing::warn!(
                    port,
                    fallback = p,
                    "port QUIC dipakai, memakai port acak: {first}"
                );
                Ok((
                    ep,
                    qos,
                    p,
                    Some(format!(
                        "Port UDP {port} tidak bisa dipakai ({first}). Tautra memakai port {p}; sesuaikan aturan firewall bila perlu."
                    )),
                ))
            }
            Err(e) => Err(NetError::new(
                app_error::NETWORK_BIND_FAILED,
                format!("Tidak bisa membuka port UDP: {e}"),
            )),
        },
        Err(e) => Err(NetError::new(
            app_error::NETWORK_BIND_FAILED,
            format!("Tidak bisa membuka port UDP: {e}"),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blocked {
    RejectedUntrusted,
    VersionMismatch,
    CertificateChanged,
    PeerPaused,
}

struct Peer {
    advert: PeerAdvert,
    next_attempt: Instant,
    backoff: Duration,
    failures: u32,
    blocked: Option<Blocked>,
}

struct ActiveSession {
    handle: SessionHandle,
    remote: SocketAddr,
    dialer: bool,
    since_unix_ms: u64,
    rtt: Option<Duration>,
}

struct DialAttempt {
    id: u64,
    device_id: Uuid,
    device_name: String,
    attempt: u32,
    task: JoinHandle<()>,
}

enum PairingState {
    Inactive,
    Hosting {
        pin: String,
        expires: Instant,
        expires_unix_ms: u64,
        attempts_left: u32,
        busy: bool,
    },
    Joining {
        device_id: Uuid,
        device_name: String,
        manual: bool,
        reply: Option<oneshot::Sender<Result<(), NetError>>>,
        task: JoinHandle<()>,
    },
    Done {
        status: PairingStatus,
        until: Instant,
    },
}

#[derive(Debug, thiserror::Error)]
enum DialError {
    #[error("perangkat tidak punya alamat yang bisa dihubungi")]
    NoAddress,
    #[error("sertifikat perangkat tidak cocok dengan yang di-pair")]
    CertificateChanged,
    #[error("tidak bisa menghubungi {0}")]
    Unreachable(String),
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
    #[error("konfigurasi TLS: {0}")]
    Config(String),
}

enum Internal {
    IncomingSession(Established),
    IncomingPairing(quinn::Connection),
    IncomingFailed {
        remote: SocketAddr,
        error: HandshakeError,
    },
    DialDone {
        attempt_id: u64,
        device_id: Uuid,
        result: Result<Established, DialError>,
    },
    HostAttemptDone(Result<PairedPeer, PairingFailure>),
    JoinDone(Result<PairedPeer, PairingFailure>),
    Networks(Result<Vec<ConnectedNetwork>, String>),
    ProbeDone {
        session_id: u64,
        results: Vec<PathProbe>,
    },
    HandoverDone {
        session_id: u64,
        result: Result<Established, DialError>,
    },
    UpdateDone {
        device_id: Uuid,
        device_name: String,
        result: Result<update::Fetched, String>,
    },
}

#[derive(Default)]
struct UpdateState {
    task: Option<JoinHandle<()>>,
    next_attempt: Option<Instant>,
    failures: u32,
    applied: bool,
}

struct PathProbe {
    addr: SocketAddr,
    rtt: Option<Duration>,
    wireless: bool,
}

impl PathProbe {
    fn score(&self) -> Option<Duration> {
        self.rtt
            .map(|rtt| rtt + if self.wireless { WIRELESS_PENALTY } else { Duration::ZERO })
    }
}

#[derive(Default)]
struct PathSwitch {
    probe_at: Option<Instant>,
    probing: Option<JoinHandle<()>>,
    pending: Option<(u64, SocketAddr)>,
    handover: Option<JoinHandle<()>>,
    probed: Vec<Ipv4Addr>,
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

struct Manager {
    local: LocalInfo,
    local_tx: watch::Sender<LocalInfo>,
    local_fp: Fingerprint,
    tls: Arc<TlsContext>,
    endpoint: Endpoint,
    port: u16,
    config: NetworkConfig,
    trust: Arc<RwLock<TrustStore>>,

    discovery: Option<Discovery>,
    discovery_gen: u64,
    discovery_retry_at: Option<Instant>,
    discovery_failed: bool,
    last_requery: Instant,
    disc_tx: mpsc::Sender<TaggedEvent>,
    peers: HashMap<Uuid, Peer>,
    fullnames: HashMap<String, Uuid>,

    session: Option<ActiveSession>,
    next_session_id: u64,
    dial: Option<DialAttempt>,
    next_attempt_id: u64,
    paused: bool,

    pairing: PairingState,
    browse_until: Option<Instant>,

    networks: Vec<NetworkInfo>,
    networks_checked: bool,
    network_allowed: bool,
    known_networks: KnownNetworks,
    qos: Option<Qos>,
    connected: Vec<ConnectedNetwork>,
    paths_tx: watch::Sender<Option<Vec<u128>>>,
    nlm_failed: bool,
    local_ipv4: Vec<LocalAddr>,
    memory: PathMemory,
    nlm_checks: VecDeque<Instant>,
    net_change_at: Option<Instant>,
    poll_fallback: bool,
    last_poll: Instant,

    path: PathSwitch,
    update: UpdateState,

    notice_log: HashMap<(&'static str, String), Instant>,
    status_tx: Option<watch::Sender<NetStatus>>,
    events_tx: broadcast::Sender<NetEvent>,
    internal_tx: mpsc::Sender<Internal>,
    session_tx: mpsc::Sender<SessionEvent>,
}

impl Manager {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<NetCommand>,
        mut internal_rx: mpsc::Receiver<Internal>,
        mut session_rx: mpsc::Receiver<SessionEvent>,
        mut disc_rx: mpsc::Receiver<TaggedEvent>,
        net_notify: Arc<Notify>,
        _watcher: Option<NetWatcher>,
    ) {
        self.evaluate();
        self.publish();
        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(cmd) => {
                        if self.handle_command(cmd).await == Flow::Exit {
                            break;
                        }
                    }
                    None => {
                        self.shutdown().await;
                        break;
                    }
                },
                Some(msg) = internal_rx.recv() => self.handle_internal(msg),
                Some(ev) = session_rx.recv() => self.handle_session_event(ev),
                Some(ev) = disc_rx.recv() => self.handle_discovery(ev),
                _ = net_notify.notified() => self.on_network_event(),
                _ = tokio::time::sleep_until(deadline.into()) => self.on_timer(),
            }
            self.evaluate();
            self.publish();
        }
        tracing::info!("manager jaringan berhenti");
    }

    fn publish(&self) {
        let status = self.build_status();
        if let Some(tx) = &self.status_tx {
            tx.send_if_modified(|current| {
                if *current != status {
                    *current = status;
                    true
                } else {
                    false
                }
            });
        }
    }

    fn build_status(&self) -> NetStatus {
        let trust = self.trust.read().expect("trust lock");
        let connection = if let Some(s) = &self.session {
            ConnectionStatus::Connected(ConnectedPeer {
                device_id: s.handle.device_id.to_string(),
                device_name: s.handle.device_name.clone(),
                address: s.remote.to_string(),
                rtt_ms: s.rtt.map(|r| r.as_secs_f32() * 1000.0),
                since_unix_ms: s.since_unix_ms,
            })
        } else if self.paused {
            ConnectionStatus::Paused
        } else if let Some(d) = &self.dial {
            ConnectionStatus::Connecting {
                device_id: d.device_id.to_string(),
                device_name: d.device_name.clone(),
                attempt: d.attempt,
            }
        } else if !self.networks_checked {
            ConnectionStatus::Idle {
                reason: IdleReason::Starting,
            }
        } else if trust.is_empty() {
            ConnectionStatus::Idle {
                reason: IdleReason::NoTrustedDevices,
            }
        } else if self.local_ipv4.is_empty() {
            ConnectionStatus::Idle {
                reason: IdleReason::NoNetwork,
            }
        } else if !self.network_allowed {
            ConnectionStatus::Idle {
                reason: IdleReason::NetworkNotAllowed,
            }
        } else if !self.config.auto_connect {
            ConnectionStatus::Idle {
                reason: IdleReason::AutoConnectDisabled,
            }
        } else {
            ConnectionStatus::Searching
        };

        let pairing = match &self.pairing {
            PairingState::Inactive => PairingStatus::Inactive,
            PairingState::Hosting {
                pin,
                expires_unix_ms,
                attempts_left,
                ..
            } => PairingStatus::Hosting {
                pin: pin.clone(),
                expires_unix_ms: *expires_unix_ms,
                attempts_left: *attempts_left,
            },
            PairingState::Joining {
                device_id,
                device_name,
                ..
            } => PairingStatus::Joining {
                device_id: device_id.to_string(),
                device_name: device_name.clone(),
            },
            PairingState::Done { status, .. } => status.clone(),
        };

        let mut discovered: Vec<DiscoveredDevice> = self
            .peers
            .values()
            .map(|p| DiscoveredDevice {
                device_id: p.advert.device_id.to_string(),
                device_name: p.advert.device_name.clone(),
                addresses: p.advert.addrs.iter().map(|a| a.to_string()).collect(),
                port: p.advert.port,
                protocol_version: p.advert.protocol_version,
                pairing_open: p.advert.pairing_open,
                trusted: trust.by_id(&p.advert.device_id).is_some(),
            })
            .collect();
        discovered.sort_by(|a, b| a.device_name.cmp(&b.device_name));

        NetStatus {
            local: LocalDevice {
                device_id: self.local.device_id.to_string(),
                device_name: self.local.device_name.clone(),
                fingerprint: self.local_fp.display(),
            },
            connection,
            network: NetworkStatus {
                allowed: self.network_allowed,
                networks: self.networks.clone(),
                quic_port: self.port,
                addresses: self
                    .local_ipv4
                    .iter()
                    .filter(|a| !a.ip.is_link_local())
                    .map(|a| a.ip.to_string())
                    .collect(),
                known: self.known_names(),
            },
            pairing,
            discovered,
            trusted: trust.entries().iter().map(TrustedEntry::to_api).collect(),
        }
    }

    fn notice(&mut self, level: NoticeLevel, code: &'static str, message: String) {
        let key = (code, message.clone());
        let now = Instant::now();
        if let Some(last) = self.notice_log.get(&key) {
            if now.duration_since(*last) < NOTICE_THROTTLE {
                return;
            }
        }
        self.notice_log.insert(key, now);
        match level {
            NoticeLevel::Error => tracing::error!(code, "{message}"),
            NoticeLevel::Warning => tracing::warn!(code, "{message}"),
            NoticeLevel::Info => tracing::info!(code, "{message}"),
        }
        let _ = self.events_tx.send(NetEvent::Notice {
            level,
            code,
            message,
        });
    }

    fn next_deadline(&self) -> Instant {
        let now = Instant::now();
        let mut deadline = now + Duration::from_secs(3600);
        let mut consider = |t: Instant| {
            if t > now && t < deadline {
                deadline = t;
            }
        };

        if self.can_dial() {
            let trust = self.trust.read().expect("trust lock");
            for peer in self.peers.values() {
                if peer.blocked.is_none() && trust.by_id(&peer.advert.device_id).is_some() {
                    consider(peer.next_attempt);
                }
            }
        }
        match &self.pairing {
            PairingState::Hosting { expires, .. } => consider(*expires),
            PairingState::Done { until, .. } => consider(*until),
            _ => {}
        }
        if let Some(t) = self.browse_until {
            consider(t);
        }
        if let Some(t) = self.nlm_checks.front() {
            consider(*t);
        }
        if let Some(t) = self.net_change_at {
            consider(t);
        }
        if let Some(t) = self.update.next_attempt {
            consider(t);
        }
        if let Some(t) = self.path.probe_at {
            consider(t);
        }
        if self.discovery.is_some() && self.session.is_none() {
            consider(self.last_requery + IDLE_REQUERY);
        }
        if let Some(t) = self.discovery_retry_at {
            consider(t);
        }
        if self.poll_fallback {
            consider(self.last_poll + NETWORK_POLL_FALLBACK);
        }
        if self.has_due_items(now) {
            return now;
        }
        deadline
    }

    fn has_due_items(&self, now: Instant) -> bool {
        self.nlm_checks.front().is_some_and(|t| *t <= now)
            || self.net_change_at.is_some_and(|t| t <= now)
            || self.path.probe_at.is_some_and(|t| t <= now)
            || self.discovery_retry_at.is_some_and(|t| t <= now)
            || (self.discovery.is_some()
                && self.session.is_none()
                && self.last_requery + IDLE_REQUERY <= now)
            || (self.poll_fallback && self.last_poll + NETWORK_POLL_FALLBACK <= now)
    }

    fn on_timer(&mut self) {
        let now = Instant::now();
        if self.net_change_at.is_some_and(|t| t <= now) {
            self.net_change_at = None;
            self.apply_network_change();
        }
        if self.path.probe_at.is_some_and(|t| t <= now) {
            self.path.probe_at = None;
            self.start_probe();
        }
        let mut nlm_due = false;
        while self.nlm_checks.front().is_some_and(|t| *t <= now) {
            self.nlm_checks.pop_front();
            nlm_due = true;
        }
        if nlm_due {
            self.spawn_nlm_check();
        }
        if self.discovery_retry_at.is_some_and(|t| t <= now) {
            self.discovery_retry_at = None;
        }
        if self.session.is_none() && self.last_requery + IDLE_REQUERY <= now {
            self.last_requery = now;
            if let Some(d) = &self.discovery {
                if let Err(e) = d.requery() {
                    tracing::warn!("query ulang mDNS gagal: {e}");
                }
            }
        }
        if self.poll_fallback && self.last_poll + NETWORK_POLL_FALLBACK <= now {
            self.last_poll = now;
            self.on_network_event();
        }
    }

    fn schedule_nlm_checks(&mut self, base: Instant, offsets_ms: &[u64]) {
        self.nlm_checks = offsets_ms
            .iter()
            .map(|ms| base + Duration::from_millis(*ms))
            .collect();
    }

    fn on_network_event(&mut self) {
        let now = Instant::now();
        if self.net_change_at.is_none() {
            self.net_change_at = Some(now + NET_CHANGE_DEBOUNCE);
        }
        self.schedule_nlm_checks(now, &NLM_RECHECK_MS);
    }

    fn apply_network_change(&mut self) {
        let ips = netwatch::local_ipv4();
        if ips == self.local_ipv4 {
            return;
        }
        tracing::info!(
            addrs = ?ips.iter().map(|a| a.ip).collect::<Vec<_>>(),
            "alamat IPv4 lokal berubah"
        );
        self.local_ipv4 = ips;
        self.recompute_allowed();
        self.path.probed.clear();
        self.schedule_probe();
        let known = self
            .session
            .as_ref()
            .and_then(|s| self.peers.get(&s.handle.device_id))
            .map(|p| p.advert.clone());
        if let Some(advert) = known {
            self.instant_switch(&advert);
        }
        if let Some(d) = self.discovery.take() {
            d.shutdown();
        }
        self.peers.clear();
        self.fullnames.clear();
        self.paused = false;
    }

    fn spawn_nlm_check(&self) {
        let tx = self.internal_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = netwatch::connected_networks().map_err(|e| e.to_string());
            let _ = tx.blocking_send(Internal::Networks(result));
        });
    }

    fn on_networks(&mut self, result: Result<Vec<ConnectedNetwork>, String>) {
        match result {
            Ok(connected) => {
                let networks: Vec<NetworkInfo> = connected.iter().map(|c| c.info.clone()).collect();
                self.connected = connected;
                if networks != self.networks {
                    tracing::info!(?networks, "jaringan tersambung");
                }
                let names: Vec<String> = networks.iter().map(|n| n.name.clone()).collect();
                self.known_networks.see(&names);
                self.networks = networks;
                self.nlm_failed = false;
            }
            Err(e) => {
                if !self.nlm_failed {
                    self.notice(
                        NoticeLevel::Warning,
                        app_error::INTERNAL,
                        format!("Tidak bisa membaca daftar jaringan Windows: {e}"),
                    );
                }
                self.nlm_failed = true;
            }
        }
        self.networks_checked = true;
        self.recompute_allowed();
    }

    fn allowed_paths(&self) -> Option<Vec<u128>> {
        if !self.config.restrict_networks {
            return None;
        }
        let mut adapters: Vec<u128> = self
            .connected
            .iter()
            .filter(|c| name_allowed(&self.config.allowed_networks, &c.info.name))
            .flat_map(|c| c.adapters.iter().copied())
            .collect();
        adapters.sort_unstable();
        adapters.dedup();
        Some(adapters)
    }

    fn path_allowed(&self, ip: Ipv4Addr) -> bool {
        self.paths_tx
            .borrow()
            .as_ref()
            .is_none_or(|adapters| adapter_allowed(adapters, ip))
    }

    fn session_addrs(&self, advert: &PeerAdvert) -> Vec<SocketAddr> {
        self.order_addrs(advert)
            .into_iter()
            .filter(|a| match a.ip() {
                IpAddr::V4(ip) => self.path_allowed(ip),
                IpAddr::V6(_) => false,
            })
            .collect()
    }

    fn known_names(&self) -> Vec<String> {
        let mut names = self.known_networks.names();
        for name in &self.config.allowed_networks {
            if !names.iter().any(|n| n.trim().eq_ignore_ascii_case(name.trim())) {
                names.push(name.clone());
            }
        }
        names
    }

    fn recompute_allowed(&mut self) {
        let paths = self.allowed_paths();
        self.paths_tx.send_if_modified(|current| {
            if *current != paths {
                *current = paths.clone();
                true
            } else {
                false
            }
        });
        if let (Some(adapters), Some(session)) = (&paths, &self.session) {
            let ok = match session.remote.ip() {
                IpAddr::V4(ip) => adapter_allowed(adapters, ip),
                IpAddr::V6(_) => false,
            };
            if !ok {
                tracing::info!(remote = %session.remote, "jalur sesi lewat jaringan yang tidak diizinkan, diputus");
                self.close_session(close_code::SUPERSEDED, b"network not allowed");
            }
        }
        let allowed = paths.as_ref().is_none_or(|adapters| {
            self.local_ipv4
                .iter()
                .any(|l| l.adapter.is_some_and(|a| adapters.contains(&a)))
        });
        if allowed != self.network_allowed {
            tracing::info!(allowed, "status izin jaringan berubah");
            self.network_allowed = allowed;
            if allowed {
                let now = Instant::now();
                let local_id = self.local.device_id;
                for peer in self.peers.values_mut() {
                    peer.next_attempt = now + dial_delay(local_id, peer.advert.device_id);
                }
            }
        }
    }

    fn advert(&self) -> LocalAdvert {
        LocalAdvert {
            device_id: self.local.device_id,
            device_name: self.local.device_name.clone(),
            port: self.port,
            pairing_open: matches!(self.pairing, PairingState::Hosting { .. }),
            wireless: netwatch::wireless_ipv4(),
            build: build::id(),
        }
    }

    fn pairing_active(&self) -> bool {
        matches!(
            self.pairing,
            PairingState::Hosting { .. } | PairingState::Joining { .. }
        ) || self.browse_until.is_some()
    }

    fn ensure_discovery(&mut self) {
        let want = !self.local_ipv4.is_empty() && (self.network_allowed || self.pairing_active());
        if want {
            let advert = self.advert();
            if let Some(d) = &mut self.discovery {
                if let Err(e) = d.update(&advert) {
                    tracing::warn!("gagal memperbarui iklan mDNS: {e}");
                }
                return;
            }
            if self.discovery_retry_at.is_some() {
                return;
            }
            self.discovery_gen += 1;
            match Discovery::start(self.discovery_gen, advert, self.disc_tx.clone()) {
                Ok(d) => {
                    self.discovery = Some(d);
                    self.last_requery = Instant::now();
                    self.discovery_failed = false;
                }
                Err(e) => {
                    if !self.discovery_failed {
                        self.notice(
                            NoticeLevel::Error,
                            app_error::DISCOVERY_FAILED,
                            format!("Discovery mDNS gagal dimulai: {e}"),
                        );
                    }
                    self.discovery_failed = true;
                    self.discovery_retry_at = Some(Instant::now() + DISCOVERY_RETRY);
                }
            }
        } else if let Some(d) = self.discovery.take() {
            d.shutdown();
            self.peers.clear();
            self.fullnames.clear();
        }
    }

    fn handle_discovery(&mut self, (generation, event): TaggedEvent) {
        if self.discovery.as_ref().map(|d| d.generation()) != Some(generation) {
            return;
        }
        match event {
            DiscoveryEvent::Resolved(advert) => {
                if advert.device_id == self.local.device_id {
                    return;
                }
                let now = Instant::now();
                let delay = dial_delay(self.local.device_id, advert.device_id);
                self.fullnames
                    .insert(advert.fullname.to_lowercase(), advert.device_id);
                {
                    let mut trust = self.trust.write().expect("trust lock");
                    if trust.rename(&advert.device_id, &advert.device_name) {
                        if let Err(e) = trust.save() {
                            tracing::warn!("gagal menyimpan nama perangkat: {e}");
                        }
                    }
                }
                let session_peer = self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.handle.device_id == advert.device_id);
                if session_peer && self.path.probed != advert.addrs {
                    self.schedule_probe();
                    self.instant_switch(&advert);
                }
                let mismatch = advert.protocol_version != PROTOCOL_VERSION
                    || advert.build != build::id();
                let trusted = self
                    .trust
                    .read()
                    .expect("trust lock")
                    .by_id(&advert.device_id)
                    .is_some();
                if mismatch && trusted {
                    self.version_notice(&advert);
                }
                match self.peers.get_mut(&advert.device_id) {
                    Some(peer) => {
                        if peer.advert.addrs != advert.addrs || peer.advert.port != advert.port {
                            peer.backoff = BACKOFF_INITIAL;
                            peer.failures = 0;
                            peer.next_attempt = now + delay;
                        }
                        if mismatch {
                            peer.blocked = Some(Blocked::VersionMismatch);
                        } else if peer.blocked == Some(Blocked::VersionMismatch) {
                            peer.blocked = None;
                            peer.failures = 0;
                            peer.backoff = BACKOFF_INITIAL;
                            peer.next_attempt = now + delay;
                        }
                        peer.advert = advert;
                    }
                    None => {
                        tracing::info!(
                            device = %advert.device_id,
                            name = %advert.device_name,
                            addrs = ?advert.addrs,
                            build = advert.build,
                            "perangkat terlihat di jaringan"
                        );
                        let blocked = mismatch.then_some(Blocked::VersionMismatch);
                        self.peers.insert(
                            advert.device_id,
                            Peer {
                                advert,
                                next_attempt: now + delay,
                                backoff: BACKOFF_INITIAL,
                                failures: 0,
                                blocked,
                            },
                        );
                    }
                }
            }
            DiscoveryEvent::Removed { fullname } => {
                if let Some(id) = self.fullnames.remove(&fullname.to_lowercase()) {
                    if let Some(peer) = self.peers.remove(&id) {
                        tracing::info!(device = %id, name = %peer.advert.device_name, "perangkat hilang dari jaringan");
                    }
                }
            }
        }
    }

    fn can_dial(&self) -> bool {
        self.session.is_none()
            && self.dial.is_none()
            && !self.paused
            && self.config.auto_connect
            && self.network_allowed
    }

    fn maybe_dial(&mut self) {
        if !self.can_dial() {
            return;
        }
        let now = Instant::now();
        let candidate = {
            let trust = self.trust.read().expect("trust lock");
            self.peers
                .values()
                .filter(|p| p.blocked.is_none() && p.next_attempt <= now)
                .filter_map(|p| trust.by_id(&p.advert.device_id).map(|e| (p, e.clone())))
                .max_by_key(|(_, e)| e.last_connected_unix_ms.unwrap_or(0))
                .map(|(p, e)| (p.advert.clone(), p.failures, e))
        };
        let Some((advert, failures, entry)) = candidate else {
            return;
        };
        let Some(fingerprint) = entry.fingerprint() else {
            return;
        };
        let addrs = self.session_addrs(&advert);
        if addrs.is_empty() {
            return;
        }
        let attempt_id = self.next_attempt_id;
        self.next_attempt_id += 1;
        let device_id = advert.device_id;
        tracing::info!(device = %device_id, attempt = failures + 1, ?addrs, "menyambungkan");

        let endpoint = self.endpoint.clone();
        let tls = self.tls.clone();
        let local = self.local.clone();
        let tx = self.internal_tx.clone();
        let task = tokio::spawn(async move {
            let result = dial_session(endpoint, tls, addrs, fingerprint, local, device_id).await;
            let _ = tx
                .send(Internal::DialDone {
                    attempt_id,
                    device_id,
                    result,
                })
                .await;
        });
        self.dial = Some(DialAttempt {
            id: attempt_id,
            device_id,
            device_name: entry.device_name.clone(),
            attempt: failures + 1,
            task,
        });
    }

    fn order_addrs(&self, advert: &PeerAdvert) -> Vec<SocketAddr> {
        let mut addrs = advert.addrs.clone();
        addrs.sort_by_key(|ip| {
            let same_subnet = self.local_ipv4.iter().any(|l| l.same_subnet(*ip));
            (!same_subnet, ip.is_link_local())
        });
        addrs
            .into_iter()
            .map(|ip| SocketAddr::V4(SocketAddrV4::new(ip, advert.port)))
            .collect()
    }

    fn on_dial_done(
        &mut self,
        attempt_id: u64,
        device_id: Uuid,
        result: Result<Established, DialError>,
    ) {
        if self.dial.as_ref().map(|d| d.id) != Some(attempt_id) {
            if let Ok(est) = result {
                est.conn.close(close_code::DUPLICATE.into(), b"stale dial");
            }
            return;
        }
        let attempt = self.dial.take().map(|d| d.attempt).unwrap_or(1);
        let err = match result {
            Ok(est) => {
                self.adopt(est);
                return;
            }
            Err(err) => err,
        };
        tracing::info!(device = %device_id, attempt, "gagal tersambung: {err}");

        let now = Instant::now();
        let local_id = self.local.device_id;
        let name = self
            .peers
            .get(&device_id)
            .map(|p| p.advert.device_name.clone())
            .unwrap_or_else(|| device_id.to_string());
        let mut notice: Option<(NoticeLevel, &'static str, String)> = None;
        let mut verify: Option<String> = None;
        let mut forget = false;
        if let Some(peer) = self.peers.get_mut(&device_id) {
            peer.failures += 1;
            match &err {
                DialError::Handshake(HandshakeError::RejectedUntrusted) => {
                    peer.blocked = Some(Blocked::RejectedUntrusted);
                    forget = true;
                }
                DialError::Handshake(HandshakeError::VersionMismatch {
                    peer_app_version,
                    peer_build,
                    ..
                }) => {
                    if peer.advert.fullname.is_empty() {
                        peer.advert.build = *peer_build;
                        peer.next_attempt = now + MANUAL_VERSION_RETRY;
                    } else {
                        peer.blocked = Some(Blocked::VersionMismatch);
                    }
                    notice = Some((
                        NoticeLevel::Warning,
                        app_error::PROTOCOL_VERSION_MISMATCH,
                        format!("{name} memakai Tautra {peer_app_version} (build {peer_build}); versi yang lebih lama akan diperbarui otomatis."),
                    ));
                }
                DialError::CertificateChanged
                | DialError::Handshake(HandshakeError::WrongDevice) => {
                    peer.blocked = Some(Blocked::CertificateChanged);
                    notice = Some((
                        NoticeLevel::Warning,
                        app_error::DEVICE_NOT_TRUSTED,
                        format!("Identitas {name} berubah sejak di-pair (aplikasi dipasang ulang?). Hapus {name} lalu pair ulang."),
                    ));
                }
                DialError::Handshake(HandshakeError::Rejected(code))
                    if *code == close_code::USER_DISCONNECT =>
                {
                    peer.blocked = Some(Blocked::PeerPaused);
                }
                DialError::Handshake(HandshakeError::Rejected(code))
                    if *code == close_code::DUPLICATE =>
                {
                    peer.next_attempt = now + BACKOFF_INITIAL;
                }
                _ => {
                    peer.next_attempt = now + peer.backoff.max(dial_delay(local_id, device_id));
                    peer.backoff = (peer.backoff * 2).min(BACKOFF_MAX);
                    if peer.failures >= 2 {
                        verify = Some(peer.advert.fullname.clone());
                    }
                }
            }
        }
        if let Some((level, code, message)) = notice {
            self.notice(level, code, message);
        }
        if forget {
            self.forget_peer(device_id, &name);
        }
        if let (Some(fullname), Some(d)) = (verify, &self.discovery) {
            d.verify(&fullname);
        }
    }

    fn prefer_new(&self, peer_id: Uuid, active_dialer: bool, new_dialer: bool) -> bool {
        let local_dials = self.local.device_id < peer_id;
        let new_preferred = new_dialer == local_dials;
        let old_preferred = active_dialer == local_dials;
        !old_preferred || new_preferred
    }

    fn adopt(&mut self, est: Established) {
        let trusted = self
            .trust
            .read()
            .expect("trust lock")
            .by_id(&est.peer_id)
            .is_some();
        if !trusted {
            est.conn.close(close_code::UNTRUSTED.into(), b"untrusted");
            return;
        }
        if let Some(active) = &self.session {
            if active.handle.device_id != est.peer_id {
                est.conn.close(close_code::DUPLICATE.into(), b"busy");
                return;
            }
            if !self.prefer_new(est.peer_id, active.dialer, est.dialer) {
                est.conn.close(close_code::DUPLICATE.into(), b"duplicate");
                return;
            }
            if let Some(old) = self.session.take() {
                tokio::spawn(async move {
                    tokio::time::sleep(HANDOVER_GRACE).await;
                    old.handle.close(close_code::SUPERSEDED, b"superseded");
                });
            }
        }
        if let Some(dial) = self.dial.take() {
            dial.task.abort();
        }

        let session_id = self.next_session_id;
        self.next_session_id += 1;
        let remote = est.remote;
        let dialer = est.dialer;
        let peer_id = est.peer_id;
        let peer_name = est.peer_name.clone();
        let handle = session::spawn(est, session_id, self.session_tx.clone());
        {
            let mut trust = self.trust.write().expect("trust lock");
            trust.mark_connected(&peer_id, &peer_name);
            if let Err(e) = trust.save() {
                tracing::warn!("gagal menyimpan daftar perangkat: {e}");
            }
        }
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.backoff = BACKOFF_INITIAL;
            peer.failures = 0;
            peer.blocked = None;
        }
        tracing::info!(device = %peer_id, name = %peer_name, %remote, dialer, session_id, "tersambung");
        self.session = Some(ActiveSession {
            handle: handle.clone(),
            remote,
            dialer,
            since_unix_ms: unix_ms(),
            rtt: None,
        });
        let _ = self.events_tx.send(NetEvent::SessionStarted(handle));
    }

    fn close_session(&mut self, code: u32, reason: &[u8]) {
        if let Some(s) = self.session.take() {
            s.handle.close(code, reason);
            let _ = self.events_tx.send(NetEvent::SessionEnded {
                device_id: s.handle.device_id,
                device_name: s.handle.device_name.clone(),
            });
        }
    }

    fn handle_session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Rtt { session_id, rtt } => {
                if let Some(s) = self.session.as_mut().filter(|s| s.handle.id == session_id) {
                    s.rtt = Some(rtt);
                }
            }
            SessionEvent::Renamed {
                session_id,
                device_name,
            } => {
                if let Some(s) = self.session.as_mut().filter(|s| s.handle.id == session_id) {
                    s.handle.device_name = device_name.clone();
                    let mut trust = self.trust.write().expect("trust lock");
                    if trust.rename(&s.handle.device_id, &device_name) {
                        if let Err(e) = trust.save() {
                            tracing::warn!("gagal menyimpan nama perangkat: {e}");
                        }
                    }
                }
            }
            SessionEvent::Ended { session_id, reason } => {
                if self.session.as_ref().map(|s| s.handle.id) != Some(session_id) {
                    return;
                }
                let Some(ended) = self.session.take() else {
                    return;
                };
                let device_id = ended.handle.device_id;
                let name = ended.handle.device_name.clone();
                let _ = self.events_tx.send(NetEvent::SessionEnded {
                    device_id,
                    device_name: name.clone(),
                });
                let now = Instant::now();
                let delay = dial_delay(self.local.device_id, device_id);
                let mut forget = false;
                if let Some(peer) = self.peers.get_mut(&device_id) {
                    match reason {
                        EndReason::PeerClosed(code) if code == close_code::USER_DISCONNECT => {
                            peer.blocked = Some(Blocked::PeerPaused);
                        }
                        EndReason::Goodbye(
                            tautra_proto::control::GoodbyeReason::UserDisconnect,
                        ) => {
                            peer.blocked = Some(Blocked::PeerPaused);
                        }
                        EndReason::PeerClosed(code) if code == close_code::UNTRUSTED => {
                            peer.blocked = Some(Blocked::RejectedUntrusted);
                            forget = true;
                        }
                        _ => {
                            peer.backoff = BACKOFF_INITIAL;
                            peer.failures = 0;
                            peer.next_attempt = now + delay;
                        }
                    }
                }
                if forget {
                    self.forget_peer(device_id, &name);
                }
            }
        }
    }

    fn forget_peer(&mut self, device_id: Uuid, name: &str) {
        let result = {
            let mut trust = self.trust.write().expect("trust lock");
            if !trust.remove(&device_id) {
                return;
            }
            trust.save()
        };
        if let Err(e) = result {
            tracing::warn!("daftar trusted gagal disimpan: {e}");
        }
        tracing::info!(device = %device_id, name, "perangkat lain menghapus pairing; ikut dihapus di sini");
        self.notice(
            NoticeLevel::Info,
            app_error::DEVICE_NOT_TRUSTED,
            format!("{name} menghapus pairing dengan perangkat ini, jadi {name} juga dihapus dari daftar di sini. Pair ulang bila ingin menyambung lagi."),
        );
    }

    fn trust_paired(&mut self, peer: &PairedPeer, manual: bool) {
        let address = match peer.remote {
            SocketAddr::V4(v4) if manual => Some(v4.to_string()),
            _ => None,
        };
        let entry = TrustedEntry {
            device_id: peer.device_id,
            device_name: peer.device_name.clone(),
            fingerprint: peer.fingerprint.to_hex(),
            paired_unix_ms: unix_ms(),
            last_connected_unix_ms: None,
            address,
        };
        let save_result = {
            let mut trust = self.trust.write().expect("trust lock");
            trust.upsert(entry);
            trust.save()
        };
        if let Err(e) = save_result {
            self.notice(
                NoticeLevel::Error,
                app_error::TRUST_STORE_FAILED,
                format!("Perangkat di-pair tetapi gagal disimpan: {e}"),
            );
        }
        tracing::info!(device = %peer.device_id, name = %peer.device_name, "pairing berhasil");
        let mut mismatched = None;
        if let Some(p) = self.peers.get_mut(&peer.device_id) {
            let mismatch = p.advert.protocol_version != PROTOCOL_VERSION
                || p.advert.build != build::id();
            p.blocked = mismatch.then_some(Blocked::VersionMismatch);
            p.failures = 0;
            p.backoff = BACKOFF_INITIAL;
            p.next_attempt = Instant::now() + dial_delay(self.local.device_id, peer.device_id);
            if mismatch {
                mismatched = Some(p.advert.clone());
            }
        }
        if let Some(advert) = mismatched {
            self.version_notice(&advert);
        }
    }

    fn pairing_done(&mut self, status: PairingStatus) {
        self.pairing = PairingState::Done {
            status,
            until: Instant::now() + PAIRING_RESULT_VISIBLE,
        };
    }

    fn on_incoming_pairing(&mut self, conn: quinn::Connection) {
        let local = self.local.clone();
        match &mut self.pairing {
            PairingState::Hosting {
                pin,
                attempts_left,
                busy,
                ..
            } if !*busy && *attempts_left > 0 => {
                *busy = true;
                let pin = pin.clone();
                let fp = self.local_fp;
                let tx = self.internal_tx.clone();
                tokio::spawn(async move {
                    let result = match tokio::time::timeout(
                        PAIRING_JOIN_TIMEOUT,
                        pairing::host(&conn, &local, fp, &pin),
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(_) => Err(PairingFailure::Timeout),
                    };
                    if result.is_err() {
                        conn.close(close_code::PAIRING_FAILED.into(), b"pairing failed");
                    }
                    let _ = tx.send(Internal::HostAttemptDone(result)).await;
                });
            }
            PairingState::Hosting { .. } | PairingState::Joining { .. } => {
                tokio::spawn(pairing::reject(conn, local, PairingRejectReason::Busy));
            }
            _ => {
                tokio::spawn(pairing::reject(
                    conn,
                    local,
                    PairingRejectReason::NotHosting,
                ));
            }
        }
    }

    fn on_host_attempt(&mut self, result: Result<PairedPeer, PairingFailure>) {
        match result {
            Ok(peer) => {
                let manual = self
                    .peers
                    .get(&peer.device_id)
                    .is_none_or(|p| p.advert.fullname.is_empty());
                self.trust_paired(&peer, manual);
                self.pairing_done(PairingStatus::Succeeded {
                    device_id: peer.device_id.to_string(),
                    device_name: peer.device_name.clone(),
                });
            }
            Err(err) => {
                tracing::info!("percobaan pairing gagal: {err}");
                let PairingState::Hosting {
                    attempts_left,
                    busy,
                    ..
                } = &mut self.pairing
                else {
                    return;
                };
                *busy = false;
                if matches!(err, PairingFailure::WrongPin) {
                    *attempts_left = attempts_left.saturating_sub(1);
                    if *attempts_left == 0 {
                        self.pairing_done(PairingStatus::Failed {
                            code: app_error::PAIRING_TOO_MANY_ATTEMPTS.into(),
                            message: "PIN salah terlalu sering; tampilkan PIN baru.".into(),
                        });
                    }
                }
            }
        }
    }

    fn on_join_done(&mut self, result: Result<PairedPeer, PairingFailure>) {
        let PairingState::Joining { reply, manual, .. } =
            std::mem::replace(&mut self.pairing, PairingState::Inactive)
        else {
            return;
        };
        match result {
            Ok(peer) => {
                self.trust_paired(&peer, manual);
                self.pairing_done(PairingStatus::Succeeded {
                    device_id: peer.device_id.to_string(),
                    device_name: peer.device_name.clone(),
                });
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
            }
            Err(failure) => {
                tracing::info!("pairing gagal: {failure}");
                let err = failure.to_net_error();
                self.pairing_done(PairingStatus::Failed {
                    code: err.code.to_string(),
                    message: err.message.clone(),
                });
                if let Some(reply) = reply {
                    let _ = reply.send(Err(err));
                }
            }
        }
    }

    fn cancel_pairing(&mut self) {
        self.browse_until = None;
        match std::mem::replace(&mut self.pairing, PairingState::Inactive) {
            PairingState::Joining { reply, task, .. } => {
                task.abort();
                if let Some(reply) = reply {
                    let _ = reply.send(Err(NetError::new(
                        app_error::PAIRING_FAILED,
                        "pairing dibatalkan",
                    )));
                }
            }
            PairingState::Hosting { busy: true, .. } => {}
            _ => {}
        }
    }

    async fn handle_command(&mut self, cmd: NetCommand) -> Flow {
        match cmd {
            NetCommand::HostPairing { reply } => {
                if matches!(self.pairing, PairingState::Joining { .. }) {
                    let _ = reply.send(Err(NetError::new(
                        app_error::PAIRING_BUSY,
                        "sedang memasukkan PIN ke perangkat lain",
                    )));
                    return Flow::Continue;
                }
                let pin = pairing::generate_pin();
                let expires_unix_ms = unix_ms() + PAIRING_WINDOW.as_millis() as u64;
                self.pairing = PairingState::Hosting {
                    pin: pin.clone(),
                    expires: Instant::now() + PAIRING_WINDOW,
                    expires_unix_ms,
                    attempts_left: PAIRING_ATTEMPTS,
                    busy: false,
                };
                tracing::info!("mode pairing aktif (menampilkan PIN)");
                let _ = reply.send(Ok(PairingHostResult {
                    pin,
                    expires_unix_ms,
                }));
            }
            NetCommand::JoinPairing {
                device_id,
                pin,
                reply,
            } => self.start_join(device_id, pin, reply),
            NetCommand::JoinPairingAt {
                addrs,
                label,
                pin,
                reply,
            } => self.begin_join(None, label, addrs, true, pin, reply),
            NetCommand::BrowseForPairing => {
                let now = Instant::now();
                self.browse_until = Some(now + PAIRING_BROWSE_WINDOW);
                if let Some(d) = &self.discovery {
                    let _ = d.requery();
                    self.last_requery = now;
                }
            }
            NetCommand::CancelPairing => self.cancel_pairing(),
            NetCommand::RemoveDevice { device_id, reply } => {
                let result = {
                    let mut trust = self.trust.write().expect("trust lock");
                    if trust.remove(&device_id) {
                        trust.save().map_err(|e| {
                            NetError::new(app_error::TRUST_STORE_FAILED, e.to_string())
                        })
                    } else {
                        Err(NetError::new(
                            app_error::DEVICE_NOT_FOUND,
                            "perangkat tidak ada di daftar trusted",
                        ))
                    }
                };
                if result.is_ok() {
                    tracing::info!(device = %device_id, "perangkat dihapus dari daftar trusted");
                    if self.session.as_ref().map(|s| s.handle.device_id) == Some(device_id) {
                        self.close_session(close_code::UNTRUSTED, b"removed");
                    }
                    if self.dial.as_ref().map(|d| d.device_id) == Some(device_id) {
                        if let Some(d) = self.dial.take() {
                            d.task.abort();
                        }
                    }
                }
                let _ = reply.send(result);
            }
            NetCommand::Disconnect => {
                tracing::info!("pengguna memutus koneksi; auto connect dijeda");
                self.paused = true;
                if let Some(d) = self.dial.take() {
                    d.task.abort();
                }
                self.close_session(close_code::USER_DISCONNECT, b"user disconnect");
            }
            NetCommand::Resume => {
                tracing::info!("auto connect dilanjutkan");
                self.paused = false;
                let now = Instant::now();
                let local_id = self.local.device_id;
                for peer in self.peers.values_mut() {
                    if peer.blocked != Some(Blocked::VersionMismatch) {
                        peer.blocked = None;
                    }
                    peer.failures = 0;
                    peer.backoff = BACKOFF_INITIAL;
                    peer.next_attempt = now + dial_delay(local_id, peer.advert.device_id);
                }
                if let Some(d) = &self.discovery {
                    let _ = d.requery();
                    self.last_requery = now;
                }
            }
            NetCommand::ForgetNetwork { name } => {
                let connected = self
                    .networks
                    .iter()
                    .any(|n| n.name.trim().eq_ignore_ascii_case(name.trim()));
                if !connected {
                    self.known_networks.forget(&name);
                }
            }
            NetCommand::UpdateConfig {
                network,
                device_name,
            } => self.update_config(network, device_name),
            NetCommand::Shutdown { reply } => {
                self.shutdown().await;
                let _ = reply.send(());
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    fn start_join(
        &mut self,
        device_id: Uuid,
        pin: String,
        reply: oneshot::Sender<Result<(), NetError>>,
    ) {
        let Some(peer) = self.peers.get(&device_id) else {
            let _ = reply.send(Err(NetError::new(
                app_error::DEVICE_NOT_FOUND,
                "perangkat tidak terlihat di jaringan",
            )));
            return;
        };
        if peer.advert.protocol_version != PROTOCOL_VERSION {
            let _ = reply.send(Err(NetError::new(
                app_error::PROTOCOL_VERSION_MISMATCH,
                format!(
                    "{} memakai versi protokol {} (perangkat ini {}); perbarui Tautra dulu di salah satu perangkat",
                    peer.advert.device_name, peer.advert.protocol_version, PROTOCOL_VERSION
                ),
            )));
            return;
        }
        let manual = peer.advert.fullname.is_empty();
        let addrs = self.order_addrs(&peer.advert);
        let device_name = peer.advert.device_name.clone();
        self.begin_join(Some(device_id), device_name, addrs, manual, pin, reply);
    }

    fn begin_join(
        &mut self,
        device_id: Option<Uuid>,
        device_name: String,
        addrs: Vec<SocketAddr>,
        manual: bool,
        pin: String,
        reply: oneshot::Sender<Result<(), NetError>>,
    ) {
        let pin = pin.trim().to_string();
        if !pairing::is_valid_pin(&pin) {
            let _ = reply.send(Err(NetError::new(
                app_error::PAIRING_INVALID_PIN,
                "PIN harus 6 angka",
            )));
            return;
        }
        if matches!(
            self.pairing,
            PairingState::Joining { .. } | PairingState::Hosting { busy: true, .. }
        ) {
            let _ = reply.send(Err(NetError::new(
                app_error::PAIRING_BUSY,
                "pairing lain sedang berlangsung",
            )));
            return;
        }
        if addrs.is_empty() {
            let _ = reply.send(Err(NetError::new(
                app_error::DEVICE_UNREACHABLE,
                "alamat perangkat tidak valid",
            )));
            return;
        }
        let endpoint = self.endpoint.clone();
        let tls = self.tls.clone();
        let local = self.local.clone();
        let fp = self.local_fp;
        let tx = self.internal_tx.clone();
        tracing::info!(device = ?device_id, name = %device_name, ?addrs, manual, "memulai pairing");
        let task = tokio::spawn(async move {
            let result = match tokio::time::timeout(
                PAIRING_JOIN_TIMEOUT,
                dial_pairing(endpoint, tls, addrs, local, fp, device_id, pin),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(PairingFailure::Timeout),
            };
            let _ = tx.send(Internal::JoinDone(result)).await;
        });
        self.pairing = PairingState::Joining {
            device_id: device_id.unwrap_or(Uuid::nil()),
            device_name,
            manual,
            reply: Some(reply),
            task,
        };
    }

    fn update_config(&mut self, network: NetworkConfig, device_name: String) {
        if device_name != self.local.device_name {
            tracing::info!(%device_name, "nama perangkat berubah");
            self.local.device_name = device_name.clone();
            self.local_tx.send_replace(self.local.clone());
            if let Some(s) = &self.session {
                s.handle.try_send_control(Control::Rename { device_name });
            }
        }
        if network.port != self.config.port {
            let rebound = std::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], network.port)))
                .and_then(|socket| {
                    let qos = Qos::new(&socket);
                    self.endpoint.rebind(socket).map(|()| qos)
                });
            match rebound {
                Ok(qos) => {
                    self.qos = qos;
                    self.port = self
                        .endpoint
                        .local_addr()
                        .map(|a| a.port())
                        .unwrap_or(network.port);
                    tracing::info!(port = self.port, "port QUIC diganti");
                }
                Err(e) => self.notice(
                    NoticeLevel::Error,
                    app_error::NETWORK_BIND_FAILED,
                    format!("Port UDP {} tidak bisa dipakai: {e}", network.port),
                ),
            }
        }
        self.config = network;
        self.recompute_allowed();
    }

    async fn shutdown(&mut self) {
        tracing::info!("mematikan jaringan");
        self.cancel_pairing();
        if let Some(d) = self.dial.take() {
            d.task.abort();
        }
        self.close_session(close_code::SHUTDOWN, b"shutdown");
        if let Some(d) = self.discovery.take() {
            d.shutdown();
        }
        self.endpoint
            .close(close_code::SHUTDOWN.into(), b"shutdown");
        let _ = tokio::time::timeout(Duration::from_secs(1), self.endpoint.wait_idle()).await;
    }

    fn schedule_probe(&mut self) {
        let at = Instant::now() + PROBE_SETTLE;
        if self.path.probe_at.is_none_or(|t| t > at) {
            self.path.probe_at = Some(at);
        }
    }

    fn start_probe(&mut self) {
        if self.path.probing.as_ref().is_some_and(|t| !t.is_finished())
            || self.path.handover.as_ref().is_some_and(|t| !t.is_finished())
        {
            self.schedule_probe();
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let peer_id = session.handle.device_id;
        if self.local.device_id >= peer_id {
            return;
        }
        let Some(peer) = self.peers.get(&peer_id) else {
            return;
        };
        let Some(fingerprint) = self
            .trust
            .read()
            .expect("trust lock")
            .by_id(&peer_id)
            .and_then(|e| e.fingerprint())
        else {
            return;
        };
        let mut addrs = self.session_addrs(&peer.advert);
        let remote_ok = match session.remote.ip() {
            IpAddr::V4(ip) => self.path_allowed(ip),
            IpAddr::V6(_) => false,
        };
        if remote_ok && !addrs.contains(&session.remote) {
            addrs.push(session.remote);
        }
        self.path.probed = peer.advert.addrs.clone();
        let wireless: Vec<bool> = addrs
            .iter()
            .map(|addr| match addr.ip() {
                IpAddr::V4(ip) => self.path_wireless(&peer.advert, ip),
                IpAddr::V6(_) => false,
            })
            .collect();
        let session_id = session.handle.id;
        let endpoint = self.endpoint.clone();
        let tls = self.tls.clone();
        let tx = self.internal_tx.clone();
        tracing::info!(?addrs, "jaringan berubah, mengukur jalur ke perangkat di background");
        self.path.probing = Some(tokio::spawn(async move {
            let mut results = Vec::with_capacity(addrs.len());
            for (addr, wireless) in addrs.into_iter().zip(wireless) {
                let rtt = probe_path(&endpoint, &tls, addr, fingerprint).await;
                results.push(PathProbe {
                    addr,
                    rtt,
                    wireless,
                });
            }
            let _ = tx.send(Internal::ProbeDone { session_id, results }).await;
        }));
    }

    fn on_probe_done(&mut self, session_id: u64, results: Vec<PathProbe>) {
        self.path.probing = None;
        let Some(session) = self.session.as_ref().filter(|s| s.handle.id == session_id) else {
            return;
        };
        let peer_id = session.handle.device_id;
        for p in &results {
            if let IpAddr::V4(ip) = p.addr.ip() {
                self.memory.record(peer_id, ip, p.score());
            }
        }
        self.memory.save();
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let summary: Vec<String> = results
            .iter()
            .map(|p| {
                let link = if p.wireless { "wifi" } else { "lan" };
                match p.rtt {
                    Some(r) => format!("{} {link} {:.2}ms", p.addr, r.as_secs_f64() * 1000.0),
                    None => format!("{} {link} gagal", p.addr),
                }
            })
            .collect();
        tracing::info!(current = %session.remote, ?summary, "hasil ukur jalur");
        let current = results
            .iter()
            .find(|p| p.addr == session.remote)
            .and_then(PathProbe::score);
        let Some((addr, best)) = results
            .iter()
            .filter_map(|p| p.score().map(|s| (p.addr, s)))
            .min_by_key(|(_, s)| *s)
        else {
            return;
        };
        if addr == session.remote {
            return;
        }
        let better = current.is_none_or(|now| best + SWITCH_MIN_GAIN < now);
        if better {
            tracing::info!(from = %session.remote, to = %addr, "jalur lebih cepat ditemukan");
            self.path.pending = Some((session_id, addr));
            self.try_switch();
        }
    }

    fn path_wireless(&self, advert: &PeerAdvert, ip: Ipv4Addr) -> bool {
        advert.wireless.contains(&ip) || netwatch::route_wireless(ip)
    }

    fn instant_switch(&mut self, advert: &PeerAdvert) {
        if self.path.handover.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let peer_id = advert.device_id;
        if session.handle.device_id != peer_id || self.local.device_id >= peer_id {
            return;
        }
        let session_id = session.handle.id;
        let remote = session.remote;
        let current = match remote.ip() {
            IpAddr::V4(ip) => {
                let penalty = if self.path_wireless(advert, ip) {
                    WIRELESS_PENALTY
                } else {
                    Duration::ZERO
                };
                session
                    .rtt
                    .map(|r| r + penalty)
                    .or_else(|| self.memory.score(peer_id, ip))
            }
            IpAddr::V6(_) => session.rtt,
        };
        let best = advert
            .addrs
            .iter()
            .copied()
            .filter(|ip| self.path_allowed(*ip))
            .filter(|ip| {
                self.local_ipv4.iter().any(|l| l.same_subnet(*ip))
                    || self.paths_tx.borrow().is_some()
            })
            .filter_map(|ip| self.memory.score(peer_id, ip).map(|s| (ip, s)))
            .min_by_key(|(_, s)| *s);
        let Some((ip, best)) = best else {
            return;
        };
        let addr = SocketAddr::V4(SocketAddrV4::new(ip, advert.port));
        if addr == remote {
            return;
        }
        if current.is_none_or(|now| best + SWITCH_MIN_GAIN < now) {
            tracing::info!(
                from = %remote,
                to = %addr,
                known_ms = best.as_secs_f64() * 1000.0,
                "jalur lebih cepat dikenal dari riwayat, langsung beralih"
            );
            self.path.pending = Some((session_id, addr));
            self.try_switch();
        }
    }

    fn try_switch(&mut self) {
        if self.path.handover.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let Some((session_id, addr)) = self.path.pending else {
            return;
        };
        let Some(session) = self.session.as_ref().filter(|s| s.handle.id == session_id) else {
            self.path.pending = None;
            return;
        };
        let peer_id = session.handle.device_id;
        let Some(fingerprint) = self
            .trust
            .read()
            .expect("trust lock")
            .by_id(&peer_id)
            .and_then(|e| e.fingerprint())
        else {
            self.path.pending = None;
            return;
        };
        self.path.pending = None;
        let endpoint = self.endpoint.clone();
        let tls = self.tls.clone();
        let local = self.local.clone();
        let tx = self.internal_tx.clone();
        self.path.handover = Some(tokio::spawn(async move {
            let result = dial_session(endpoint, tls, vec![addr], fingerprint, local, peer_id).await;
            let _ = tx.send(Internal::HandoverDone { session_id, result }).await;
        }));
    }

    fn on_handover_done(&mut self, session_id: u64, result: Result<Established, DialError>) {
        self.path.handover = None;
        match result {
            Ok(est) => {
                if self.session.as_ref().map(|s| s.handle.id) != Some(session_id) {
                    est.conn.close(close_code::DUPLICATE.into(), b"stale handover");
                    return;
                }
                tracing::info!(to = %est.remote, "beralih ke jalur yang lebih cepat");
                self.adopt(est);
            }
            Err(e) => tracing::info!("pindah jalur dibatalkan: {e}"),
        }
    }

    fn handle_internal(&mut self, msg: Internal) {
        match msg {
            Internal::IncomingSession(est) => {
                if self.paused {
                    est.conn
                        .close(close_code::USER_DISCONNECT.into(), b"paused");
                    return;
                }
                self.adopt(est);
            }
            Internal::IncomingPairing(conn) => self.on_incoming_pairing(conn),
            Internal::IncomingFailed { remote, error } => {
                tracing::info!(%remote, "koneksi masuk ditolak: {error}");
                if let HandshakeError::VersionMismatch {
                    peer_app_version,
                    peer_build,
                    ..
                } = &error
                {
                    if let std::net::IpAddr::V4(ip) = remote.ip() {
                        for peer in self.peers.values_mut() {
                            if peer.advert.fullname.is_empty() && peer.advert.addrs.contains(&ip) {
                                peer.advert.build = *peer_build;
                            }
                        }
                    }
                    self.notice(
                        NoticeLevel::Warning,
                        app_error::PROTOCOL_VERSION_MISMATCH,
                        format!("Perangkat di {remote} memakai Tautra {peer_app_version} (build {peer_build}); versi yang lebih lama akan diperbarui otomatis."),
                    );
                }
            }
            Internal::DialDone {
                attempt_id,
                device_id,
                result,
            } => self.on_dial_done(attempt_id, device_id, result),
            Internal::HostAttemptDone(result) => self.on_host_attempt(result),
            Internal::JoinDone(result) => self.on_join_done(result),
            Internal::Networks(result) => self.on_networks(result),
            Internal::ProbeDone {
                session_id,
                results,
            } => self.on_probe_done(session_id, results),
            Internal::HandoverDone { session_id, result } => {
                self.on_handover_done(session_id, result)
            }
            Internal::UpdateDone {
                device_id,
                device_name,
                result,
            } => self.on_update_done(device_id, device_name, result),
        }
    }

    fn evaluate(&mut self) {
        let now = Instant::now();
        match &self.pairing {
            PairingState::Hosting {
                expires,
                busy: false,
                ..
            } if now >= *expires => {
                self.pairing_done(PairingStatus::Failed {
                    code: app_error::PAIRING_TIMEOUT.into(),
                    message: "PIN kedaluwarsa".into(),
                });
            }
            PairingState::Done { until, .. } if now >= *until => {
                self.pairing = PairingState::Inactive;
            }
            _ => {}
        }
        if self.browse_until.is_some_and(|t| now >= t) {
            self.browse_until = None;
        }
        let remote = self.session.as_ref().map(|s| s.remote);
        if let Some(qos) = &mut self.qos {
            qos.keep_only(remote);
        }
        self.ensure_discovery();
        self.ensure_manual_peers();
        self.maybe_dial();
        self.maybe_update();
    }

    fn ensure_manual_peers(&mut self) {
        if self.local_ipv4.is_empty() {
            return;
        }
        let manual: Vec<(Uuid, String, SocketAddrV4)> = {
            let trust = self.trust.read().expect("trust lock");
            trust
                .entries()
                .iter()
                .filter(|e| !self.peers.contains_key(&e.device_id))
                .filter_map(|e| {
                    e.manual_address()
                        .map(|a| (e.device_id, e.device_name.clone(), a))
                })
                .collect()
        };
        let now = Instant::now();
        for (device_id, device_name, addr) in manual {
            tracing::info!(device = %device_id, name = %device_name, %addr, "memakai alamat manual");
            self.peers.insert(
                device_id,
                Peer {
                    advert: PeerAdvert {
                        fullname: String::new(),
                        device_id,
                        device_name,
                        addrs: vec![*addr.ip()],
                        port: addr.port(),
                        protocol_version: PROTOCOL_VERSION,
                        pairing_open: false,
                        wireless: Vec::new(),
                        build: 0,
                    },
                    next_attempt: now + dial_delay(self.local.device_id, device_id),
                    backoff: BACKOFF_INITIAL,
                    failures: 0,
                    blocked: None,
                },
            );
        }
    }

    fn version_notice(&mut self, advert: &PeerAdvert) {
        let local = build::id();
        let message = if advert.protocol_version != PROTOCOL_VERSION {
            format!(
                "{} memakai protokol {} (perangkat ini {}); versi yang lebih lama akan diperbarui otomatis.",
                advert.device_name, advert.protocol_version, PROTOCOL_VERSION
            )
        } else if advert.build > local {
            format!(
                "{} memakai Tautra yang lebih baru (build {}); perangkat ini akan memperbarui diri otomatis.",
                advert.device_name, advert.build
            )
        } else {
            format!(
                "{} masih memakai Tautra lama (build {}); perangkat itu akan memperbarui diri dari perangkat ini.",
                advert.device_name, advert.build
            )
        };
        self.notice(NoticeLevel::Info, app_error::PROTOCOL_VERSION_MISMATCH, message);
    }

    fn maybe_update(&mut self) {
        if self.update.applied
            || self.update.task.as_ref().is_some_and(|t| !t.is_finished())
            || self.update.next_attempt.is_some_and(|t| t > Instant::now())
        {
            return;
        }
        let local = build::id();
        if local == 0 {
            return;
        }
        let candidate = {
            let trust = self.trust.read().expect("trust lock");
            self.peers
                .values()
                .filter(|p| p.advert.build > local)
                .filter_map(|p| {
                    trust
                        .by_id(&p.advert.device_id)
                        .and_then(|e| e.fingerprint())
                        .map(|fp| (p.advert.clone(), fp))
                })
                .max_by_key(|(a, _)| a.build)
        };
        let Some((advert, fingerprint)) = candidate else {
            return;
        };
        let addrs = self.session_addrs(&advert);
        if addrs.is_empty() {
            return;
        }
        let device_id = advert.device_id;
        let device_name = advert.device_name.clone();
        let expected_build = advert.build;
        tracing::info!(device = %device_id, name = %device_name, build = expected_build, ?addrs, "mengunduh pembaruan dari perangkat lain");
        if self.update.failures == 0 {
            self.notice(
                NoticeLevel::Info,
                app_error::UPDATE_RUNNING,
                format!("Memperbarui Tautra dari {device_name} (build {expected_build})..."),
            );
        }
        let endpoint = self.endpoint.clone();
        let tls = self.tls.clone();
        let tx = self.internal_tx.clone();
        self.update.next_attempt = None;
        self.update.task = Some(tokio::spawn(async move {
            let result = update::fetch(endpoint, tls, addrs, fingerprint, expected_build).await;
            let _ = tx
                .send(Internal::UpdateDone {
                    device_id,
                    device_name,
                    result,
                })
                .await;
        }));
    }

    fn on_update_done(
        &mut self,
        device_id: Uuid,
        device_name: String,
        result: Result<update::Fetched, String>,
    ) {
        self.update.task = None;
        match result {
            Ok(fetched) => {
                tracing::info!(device = %device_id, build = fetched.build, files = fetched.files.len(), "pembaruan terunduh dan terverifikasi");
                self.update.applied = true;
                self.update.failures = 0;
                let _ = self.events_tx.send(NetEvent::UpdateReady {
                    device_name,
                    build: fetched.build,
                    dir: fetched.dir,
                    files: fetched.files,
                });
            }
            Err(e) => {
                self.update.failures += 1;
                let wait = (UPDATE_RETRY_MIN * 2u32.pow(self.update.failures.min(6) - 1))
                    .min(UPDATE_RETRY_MAX);
                self.update.next_attempt = Some(Instant::now() + wait);
                self.notice(
                    NoticeLevel::Error,
                    app_error::UPDATE_FAILED,
                    format!(
                        "Pembaruan dari {device_name} gagal: {e}. Dicoba lagi dalam {} detik.",
                        wait.as_secs()
                    ),
                );
            }
        }
    }
}

fn dial_delay(local: Uuid, peer: Uuid) -> Duration {
    if local < peer {
        Duration::ZERO
    } else {
        NON_PREFERRED_DIAL_DELAY
    }
}

fn adapter_allowed(adapters: &[u128], ip: Ipv4Addr) -> bool {
    netwatch::route_adapter(ip).is_some_and(|a| adapters.contains(&a))
}

fn name_allowed(allowed: &[String], name: &str) -> bool {
    allowed
        .iter()
        .any(|a| a.trim().eq_ignore_ascii_case(name.trim()))
}


async fn accept_loop(
    endpoint: Endpoint,
    local_rx: watch::Receiver<LocalInfo>,
    trust: Arc<RwLock<TrustStore>>,
    tx: mpsc::Sender<Internal>,
    paths: watch::Receiver<Option<Vec<u128>>>,
) {
    while let Some(incoming) = endpoint.accept().await {
        let remote_ip = incoming.remote_address().ip();
        let refused = match &*paths.borrow() {
            None => false,
            Some(adapters) => match remote_ip {
                IpAddr::V4(ip) => !adapter_allowed(adapters, ip),
                IpAddr::V6(_) => true,
            },
        };
        if refused {
            tracing::debug!(remote = %remote_ip, "koneksi masuk lewat jaringan yang tidak diizinkan ditolak");
            incoming.refuse();
            continue;
        }
        let local = local_rx.borrow().clone();
        let trust = trust.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let remote = incoming.remote_address();
            let connecting = match incoming.accept() {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(%remote, "koneksi masuk gagal: {e}");
                    return;
                }
            };
            let conn = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => {
                    tracing::debug!(%remote, "handshake QUIC masuk gagal: {e}");
                    return;
                }
                Err(_) => return,
            };
            let msg = match tls::negotiated_alpn(&conn).as_deref() {
                Some(alpn) if alpn == ALPN_PAIRING => Internal::IncomingPairing(conn),
                Some(alpn) if alpn == ALPN_PROBE => {
                    let _ = tokio::time::timeout(PROBE_HOLD, conn.closed()).await;
                    conn.close(close_code::NORMAL.into(), b"probe");
                    return;
                }
                Some(alpn) if alpn == ALPN_UPDATE => {
                    let trusted = tls::peer_fingerprint(&conn).is_some_and(|fp| {
                        trust
                            .read()
                            .ok()
                            .is_some_and(|store| store.by_fingerprint(&fp).is_some())
                    });
                    if !trusted {
                        conn.close(close_code::UNTRUSTED.into(), b"untrusted");
                        return;
                    }
                    tracing::info!(%remote, "perangkat lain meminta pembaruan");
                    update::serve(conn).await;
                    return;
                }
                Some(alpn) if alpn == ALPN_SESSION => {
                    match tokio::time::timeout(
                        HANDSHAKE_TIMEOUT,
                        session::handshake_acceptor(conn.clone(), &local, &trust),
                    )
                    .await
                    {
                        Ok(Ok(est)) => Internal::IncomingSession(est),
                        Ok(Err(error)) => Internal::IncomingFailed { remote, error },
                        Err(_) => {
                            conn.close(close_code::PROTOCOL_ERROR.into(), b"timeout");
                            Internal::IncomingFailed {
                                remote,
                                error: HandshakeError::Timeout,
                            }
                        }
                    }
                }
                _ => {
                    conn.close(close_code::PROTOCOL_ERROR.into(), b"alpn");
                    return;
                }
            };
            let _ = tx.send(msg).await;
        });
    }
}

async fn probe_path(
    endpoint: &Endpoint,
    tls: &TlsContext,
    addr: SocketAddr,
    expected: Fingerprint,
) -> Option<Duration> {
    let (config, _) = tls.client_config(ALPN_PROBE, Some(expected)).ok()?;
    let mut samples: Vec<Duration> = Vec::with_capacity(PROBE_ROUNDS);
    for _ in 0..PROBE_ROUNDS {
        let Ok(connecting) = endpoint.connect_with(config.clone(), addr, SERVER_NAME) else {
            break;
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(conn)) => {
                samples.push(conn.rtt());
                conn.close(close_code::NORMAL.into(), b"probe");
            }
            _ => break,
        }
    }
    samples.sort();
    samples.get(samples.len() / 2).copied()
}

async fn dial_session(
    endpoint: Endpoint,
    tls: Arc<TlsContext>,
    addrs: Vec<SocketAddr>,
    expected: Fingerprint,
    local: LocalInfo,
    expected_id: Uuid,
) -> Result<Established, DialError> {
    let (config, mismatch) = tls
        .client_config(ALPN_SESSION, Some(expected))
        .map_err(|e| DialError::Config(e.to_string()))?;
    let mut last = DialError::NoAddress;
    for addr in addrs {
        let connecting = match endpoint.connect_with(config.clone(), addr, SERVER_NAME) {
            Ok(c) => c,
            Err(e) => {
                last = DialError::Unreachable(format!("{addr} ({e})"));
                continue;
            }
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(conn)) => {
                return match tokio::time::timeout(
                    HANDSHAKE_TIMEOUT,
                    session::handshake_dialer(conn.clone(), &local, expected_id),
                )
                .await
                {
                    Ok(result) => result.map_err(DialError::from),
                    Err(_) => {
                        conn.close(close_code::PROTOCOL_ERROR.into(), b"timeout");
                        Err(DialError::Handshake(HandshakeError::Timeout))
                    }
                };
            }
            Ok(Err(e)) => {
                if mismatch.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(DialError::CertificateChanged);
                }
                last = DialError::Unreachable(format!("{addr} ({e})"));
            }
            Err(_) => last = DialError::Unreachable(format!("{addr} (timeout)")),
        }
    }
    Err(last)
}

async fn dial_pairing(
    endpoint: Endpoint,
    tls: Arc<TlsContext>,
    addrs: Vec<SocketAddr>,
    local: LocalInfo,
    local_fp: Fingerprint,
    host_id: Option<Uuid>,
    pin: String,
) -> Result<PairedPeer, PairingFailure> {
    let (config, _) = tls
        .client_config(ALPN_PAIRING, None)
        .map_err(|e| PairingFailure::Protocol(e.to_string()))?;
    let mut last = String::from("tidak ada alamat");
    for addr in addrs {
        let connecting = match endpoint.connect_with(config.clone(), addr, SERVER_NAME) {
            Ok(c) => c,
            Err(e) => {
                last = format!("{addr} ({e})");
                continue;
            }
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(conn)) => {
                let result = pairing::join(&conn, &local, local_fp, host_id, &pin).await;
                if result.is_err() {
                    conn.close(close_code::PAIRING_FAILED.into(), b"pairing failed");
                }
                return result;
            }
            Ok(Err(e)) => last = format!("{addr} ({e})"),
            Err(_) => last = format!("{addr} (timeout)"),
        }
    }
    Err(PairingFailure::Unreachable(last))
}
