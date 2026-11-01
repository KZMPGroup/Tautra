export interface GeneralConfig {
  device_name: string;
  language: string;
  autostart: boolean;
}

export interface NetworkConfig {
  port: number;
  auto_connect: boolean;
  restrict_networks: boolean;
  allowed_networks: string[];
}

export interface InputConfig {
  edge_delay_ms: number;
  hotkey_switch_local: string;
  hotkey_switch_remote: string;
  hotkey_lock: string;
  hotkey_gaming: string;
  gaming_auto: boolean;
}

export interface ServiceStatus {
  installed: boolean;
  running: boolean;
}

export interface ShareConfig {
  clipboard: boolean;
  text: boolean;
  html: boolean;
  rtf: boolean;
  image: boolean;
  files: boolean;
  drag_drop: boolean;
  max_clipboard_mb: number;
}

export interface Config {
  general: GeneralConfig;
  network: NetworkConfig;
  input: InputConfig;
  share: ShareConfig;
}

export interface LocalDevice {
  device_id: string;
  device_name: string;
  fingerprint: string;
}

export type IdleReason =
  | "starting"
  | "no_trusted_devices"
  | "no_network"
  | "network_not_allowed"
  | "auto_connect_disabled";

export interface ConnectedPeer {
  device_id: string;
  device_name: string;
  address: string;
  rtt_ms: number | null;
  since_unix_ms: number;
}

export type ConnectionStatus =
  | { state: "idle"; reason: IdleReason }
  | { state: "searching" }
  | { state: "connecting"; device_id: string; device_name: string; attempt: number }
  | ({ state: "connected" } & ConnectedPeer)
  | { state: "paused" }
  | { state: "error"; code: string; message: string };

export type NetworkCategory = "public" | "private" | "domain";

export interface NetworkInfo {
  name: string;
  category: NetworkCategory;
}

export interface NetworkStatus {
  allowed: boolean;
  networks: NetworkInfo[];
  quic_port: number;
  addresses: string[];
  known: string[];
}

export type PairingStatus =
  | { state: "inactive" }
  | { state: "hosting"; pin: string; expires_unix_ms: number; attempts_left: number }
  | { state: "joining"; device_id: string; device_name: string }
  | { state: "succeeded"; device_id: string; device_name: string }
  | { state: "failed"; code: string; message: string };

export interface DiscoveredDevice {
  device_id: string;
  device_name: string;
  addresses: string[];
  port: number;
  protocol_version: number;
  pairing_open: boolean;
  trusted: boolean;
}

export interface TrustedDevice {
  device_id: string;
  device_name: string;
  fingerprint: string;
  paired_unix_ms: number;
  last_connected_unix_ms: number | null;
  address: string | null;
}

export type NoticeLevel = "info" | "warning" | "error";

export interface Notice {
  id: number;
  level: NoticeLevel;
  code: string;
  message: string;
  at_unix_ms: number;
}

export interface AgentStatus {
  agent_version: string;
  build: number;
  protocol_version: number;
  started_unix_ms: number;
  local_device: LocalDevice;
  connection: ConnectionStatus;
  network: NetworkStatus;
  pairing: PairingStatus;
  discovered: DiscoveredDevice[];
  trusted: TrustedDevice[];
  kvm: KvmStatus;
  share: ShareStatus;
  notices: Notice[];
}

export interface PairingHostResult {
  pin: string;
  expires_unix_ms: number;
}

export interface DiagnosticsInfo {
  agent_pid: number;
  config_path: string;
  log_dir: string;
  data_dir: string;
  cpu_percent: number | null;
  working_set_mb: number | null;
  private_mb: number | null;
}

export interface CompatReport {
  os_version: string;
  arch: string;
  supported: boolean;
  warnings: string[];
}

export interface LinkSnapshot {
  connected: boolean;
  status: AgentStatus | null;
  config: Config | null;
  error: string | null;
}

export interface UiInfo {
  mica: boolean;
  version: string;
  languages: string[];
}

export interface UiError {
  code: string;
  message: string;
}

export type KvmState = "inactive" | "waiting" | "ready";
export type KvmRole = "input_host" | "remote";
export type KvmActive = "local" | "remote";

export interface LayoutMonitor {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale_percent: number;
  diagonal_inch: number | null;
  primary: boolean;
}

export interface LayoutDevice {
  device_id: string;
  device_name: string;
  is_local: boolean;
  portable: boolean;
  x: number;
  y: number;
  monitors: LayoutMonitor[];
}

export interface LayoutView {
  revision: number;
  input_host: string;
  devices: LayoutDevice[];
}

export interface KvmStatus {
  state: KvmState;
  role: KvmRole | null;
  active: KvmActive;
  controlled_by_peer: boolean;
  locked: boolean;
  gaming: boolean;
  layout: LayoutView | null;
}

export interface LayoutPosition {
  device_id: string;
  x: number;
  y: number;
}

export type ClipKind = "text" | "html" | "rtf" | "image" | "files";

export interface ClipActivity {
  at_unix_ms: number;
  kinds: ClipKind[];
  bytes: number;
  files: number;
}

export type TransferDirection = "incoming" | "outgoing";
export type TransferState = "running" | "waiting" | "done" | "failed" | "cancelled";

export interface TransferView {
  id: number;
  direction: TransferDirection;
  peer_name: string;
  name: string;
  items: number;
  files: number;
  total_bytes: number;
  done_bytes: number;
  state: TransferState;
  error: string | null;
  started_unix_ms: number;
  finished_unix_ms: number | null;
  path: string | null;
}

export interface ShareStatus {
  clipboard_ready: boolean;
  drag_ready: boolean;
  last_sent: ClipActivity | null;
  last_received: ClipActivity | null;
  transfers: TransferView[];
}
