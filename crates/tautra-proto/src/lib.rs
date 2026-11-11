pub mod control;
pub mod frame;
pub mod kvm;
pub mod pairing;
pub mod share;
pub mod update;

pub const PROTOCOL_VERSION: u32 = 12;

pub const ALPN_SESSION: &[u8] = b"tautra";
pub const ALPN_PAIRING: &[u8] = b"tautra-pair";
pub const ALPN_PROBE: &[u8] = b"tautra-probe";
pub const ALPN_UPDATE: &[u8] = b"tautra-update";

pub const MDNS_SERVICE_TYPE: &str = "_tautra._udp.local.";

pub mod txt {
    pub const DEVICE_ID: &str = "id";
    pub const NAME: &str = "name";
    pub const PROTOCOL: &str = "pv";
    pub const PAIRING: &str = "pair";
    pub const WIRELESS: &str = "wl";
    pub const BUILD: &str = "bv";
}

pub const MAX_DEVICE_NAME: usize = 63;

pub type DeviceId = [u8; 16];

pub mod close_code {
    pub const NORMAL: u32 = 0;
    pub const UNTRUSTED: u32 = 1;
    pub const VERSION_MISMATCH: u32 = 2;
    pub const DUPLICATE: u32 = 3;
    pub const PAIRING_FAILED: u32 = 4;
    pub const SHUTDOWN: u32 = 5;
    pub const USER_DISCONNECT: u32 = 6;
    pub const PROTOCOL_ERROR: u32 = 7;
    pub const HEARTBEAT_TIMEOUT: u32 = 8;
    pub const PAIRING_DONE: u32 = 9;
    pub const SUPERSEDED: u32 = 10;
}
