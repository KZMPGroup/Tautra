use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::os::windows::io::AsRawSocket;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::QoS::{
    QOSAddSocketToFlow, QOSCloseHandle, QOSCreateHandle, QOSRemoveSocketFromFlow, QOSTrafficTypeVoice,
    QOS_NON_ADAPTIVE_FLOW, QOS_VERSION,
};
use windows::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, IN_ADDR_0, SOCKADDR, SOCKADDR_IN, SOCKET,
};

pub struct Qos {
    handle: HANDLE,
    socket: SOCKET,
    flows: HashMap<SocketAddr, u32>,
    failed: bool,
}

unsafe impl Send for Qos {}

impl Qos {
    pub fn new(socket: &UdpSocket) -> Option<Self> {
        let version = QOS_VERSION {
            MajorVersion: 1,
            MinorVersion: 0,
        };
        let mut handle = HANDLE::default();
        if !unsafe { QOSCreateHandle(&version, &mut handle) }.as_bool() {
            tracing::warn!(
                "QoS Windows tidak tersedia: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }
        Some(Self {
            handle,
            socket: SOCKET(socket.as_raw_socket() as usize),
            flows: HashMap::new(),
            failed: false,
        })
    }

    pub fn keep_only(&mut self, keep: Option<SocketAddr>) {
        let stale: Vec<SocketAddr> = self
            .flows
            .keys()
            .filter(|a| Some(**a) != keep)
            .copied()
            .collect();
        for addr in stale {
            if let Some(id) = self.flows.remove(&addr) {
                unsafe {
                    let _ = QOSRemoveSocketFromFlow(self.handle, Some(self.socket), id, None);
                }
            }
        }
        let Some(addr) = keep else {
            return;
        };
        if self.flows.contains_key(&addr) {
            return;
        }
        let SocketAddr::V4(v4) = addr else {
            return;
        };
        let dest = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: v4.port().to_be(),
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
            },
            sin_zero: [0; 8],
        };
        let mut id = 0u32;
        let ok = unsafe {
            QOSAddSocketToFlow(
                self.handle,
                self.socket,
                Some(&dest as *const SOCKADDR_IN as *const SOCKADDR),
                QOSTrafficTypeVoice,
                Some(QOS_NON_ADAPTIVE_FLOW),
                &mut id,
            )
        }
        .as_bool();
        if ok {
            tracing::info!(%addr, "lalu lintas ke perangkat ditandai prioritas tinggi (QoS)");
            self.flows.insert(addr, id);
            self.failed = false;
        } else if !self.failed {
            self.failed = true;
            tracing::warn!(
                %addr,
                "prioritas QoS gagal dipasang: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

impl Drop for Qos {
    fn drop(&mut self) {
        unsafe {
            for id in self.flows.values() {
                let _ = QOSRemoveSocketFromFlow(self.handle, Some(self.socket), *id, None);
            }
            let _ = QOSCloseHandle(self.handle);
        }
    }
}
