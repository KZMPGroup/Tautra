use std::ffi::c_void;
use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;

use tautra_ipc::api::{NetworkCategory, NetworkInfo};
use tokio::sync::Notify;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToGuid, GetBestInterfaceEx, GetIfEntry2, NotifyIpInterfaceChange,
    NotifyUnicastIpAddressChange, IF_TYPE_IEEE80211, MIB_IF_ROW2, MIB_IPINTERFACE_ROW,
    MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW,
};
use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, NetworkListManager, NLM_ENUM_NETWORK_CONNECTED,
    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED, NLM_NETWORK_CATEGORY_PRIVATE,
};
use windows::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows::Win32::Networking::WinSock::{
    AF_INET, AF_UNSPEC, IN_ADDR, IN_ADDR_0, SOCKADDR, SOCKADDR_IN,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

pub struct NetWatcher {
    iface_handle: HANDLE,
    addr_handle: HANDLE,
    ctx: *const Notify,
}

unsafe impl Send for NetWatcher {}

impl NetWatcher {
    pub fn start(notify: Arc<Notify>) -> io::Result<Self> {
        let ctx = Arc::into_raw(notify);
        let mut iface_handle = HANDLE::default();
        let mut addr_handle = HANDLE::default();
        unsafe {
            if let Err(e) = NotifyIpInterfaceChange(
                AF_UNSPEC,
                Some(on_interface_change),
                Some(ctx.cast::<c_void>()),
                false,
                &mut iface_handle,
            )
            .ok()
            {
                drop(Arc::from_raw(ctx));
                return Err(io::Error::other(format!("NotifyIpInterfaceChange: {e}")));
            }
            if let Err(e) = NotifyUnicastIpAddressChange(
                AF_INET,
                Some(on_address_change),
                Some(ctx.cast::<c_void>()),
                false,
                &mut addr_handle,
            )
            .ok()
            {
                let _ = CancelMibChangeNotify2(iface_handle);
                drop(Arc::from_raw(ctx));
                return Err(io::Error::other(format!(
                    "NotifyUnicastIpAddressChange: {e}"
                )));
            }
        }
        Ok(Self {
            iface_handle,
            addr_handle,
            ctx,
        })
    }
}

impl Drop for NetWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelMibChangeNotify2(self.iface_handle);
            let _ = CancelMibChangeNotify2(self.addr_handle);
            drop(Arc::from_raw(self.ctx));
        }
    }
}

unsafe extern "system" fn on_interface_change(
    ctx: *const c_void,
    _row: *const MIB_IPINTERFACE_ROW,
    _kind: MIB_NOTIFICATION_TYPE,
) {
    if let Some(notify) = (ctx as *const Notify).as_ref() {
        notify.notify_one();
    }
}

unsafe extern "system" fn on_address_change(
    ctx: *const c_void,
    _row: *const MIB_UNICASTIPADDRESS_ROW,
    _kind: MIB_NOTIFICATION_TYPE,
) {
    if let Some(notify) = (ctx as *const Notify).as_ref() {
        notify.notify_one();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedNetwork {
    pub info: NetworkInfo,
    pub adapters: Vec<u128>,
}

pub fn connected_networks() -> windows::core::Result<Vec<ConnectedNetwork>> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = (|| {
            let nlm: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
            let networks = nlm.GetNetworks(NLM_ENUM_NETWORK_CONNECTED)?;
            let mut out = Vec::new();
            loop {
                let mut item = [None];
                let mut fetched = 0u32;
                networks.Next(&mut item, Some(&mut fetched))?;
                if fetched == 0 {
                    break;
                }
                let Some(network) = item[0].take() else {
                    break;
                };
                let name = network.GetName().map(|b| b.to_string()).unwrap_or_default();
                let category = match network.GetCategory()? {
                    NLM_NETWORK_CATEGORY_PRIVATE => NetworkCategory::Private,
                    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED => NetworkCategory::Domain,
                    _ => NetworkCategory::Public,
                };
                let mut adapters = Vec::new();
                if let Ok(connections) = network.GetNetworkConnections() {
                    loop {
                        let mut conn = [None];
                        let mut got = 0u32;
                        if connections.Next(&mut conn, Some(&mut got)).is_err() || got == 0 {
                            break;
                        }
                        let Some(conn) = conn[0].take() else {
                            break;
                        };
                        if let Ok(id) = conn.GetAdapterId() {
                            adapters.push(id.to_u128());
                        }
                    }
                }
                out.push(ConnectedNetwork {
                    info: NetworkInfo { name, category },
                    adapters,
                });
            }
            out.sort_by(|a, b| a.info.name.cmp(&b.info.name));
            Ok(out)
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalAddr {
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub adapter: Option<u128>,
}

fn adapter_guid(index: u32) -> Option<u128> {
    let mut luid = NET_LUID_LH::default();
    if unsafe { ConvertInterfaceIndexToLuid(index, &mut luid) }.is_err() {
        return None;
    }
    let mut guid = windows::core::GUID::zeroed();
    if unsafe { ConvertInterfaceLuidToGuid(&luid, &mut guid) }.is_err() {
        return None;
    }
    Some(guid.to_u128())
}

impl LocalAddr {
    pub fn same_subnet(&self, other: Ipv4Addr) -> bool {
        let mask = u32::from(self.netmask);
        (u32::from(self.ip) & mask) == (u32::from(other) & mask)
    }
}

pub fn local_ipv4() -> Vec<LocalAddr> {
    let mut out: Vec<LocalAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() => Some(LocalAddr {
                ip: v4.ip,
                netmask: v4.netmask,
                adapter: iface.index.and_then(adapter_guid),
            }),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn interface_wireless(index: u32) -> bool {
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    unsafe { GetIfEntry2(&mut row) }.is_ok() && row.Type == IF_TYPE_IEEE80211
}

pub fn wireless_ipv4() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4)
                if !v4.ip.is_loopback() && iface.index.is_some_and(interface_wireless) =>
            {
                Some(v4.ip)
            }
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn route_interface(dest: Ipv4Addr) -> Option<u32> {
    let addr = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: 0,
        sin_addr: IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: u32::from_ne_bytes(dest.octets()),
            },
        },
        sin_zero: [0; 8],
    };
    let mut index = 0u32;
    let found = unsafe {
        GetBestInterfaceEx(&addr as *const SOCKADDR_IN as *const SOCKADDR, &mut index)
    };
    (found == 0).then_some(index)
}

pub fn route_adapter(dest: Ipv4Addr) -> Option<u128> {
    route_interface(dest).and_then(adapter_guid)
}

pub fn route_wireless(dest: Ipv4Addr) -> bool {
    let addr = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: 0,
        sin_addr: IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: u32::from_ne_bytes(dest.octets()),
            },
        },
        sin_zero: [0; 8],
    };
    let mut index = 0u32;
    let found = unsafe {
        GetBestInterfaceEx(&addr as *const SOCKADDR_IN as *const SOCKADDR, &mut index)
    };
    found == 0 && interface_wireless(index)
}
