use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use tautra_proto::{txt, MDNS_SERVICE_TYPE};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAdvert {
    pub device_id: Uuid,
    pub device_name: String,
    pub port: u16,
    pub pairing_open: bool,
    pub wireless: Vec<Ipv4Addr>,
    pub build: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAdvert {
    pub fullname: String,
    pub device_id: Uuid,
    pub device_name: String,
    pub addrs: Vec<Ipv4Addr>,
    pub port: u16,
    pub protocol_version: u32,
    pub pairing_open: bool,
    pub wireless: Vec<Ipv4Addr>,
    pub build: u64,
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Resolved(PeerAdvert),
    Removed { fullname: String },
}

pub type TaggedEvent = (u64, DiscoveryEvent);

pub struct Discovery {
    daemon: ServiceDaemon,
    generation: u64,
    own_fullname: String,
    advert: LocalAdvert,
    events: mpsc::Sender<TaggedEvent>,
}

impl Discovery {
    pub fn start(
        generation: u64,
        advert: LocalAdvert,
        events: mpsc::Sender<TaggedEvent>,
    ) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        daemon.disable_interface(IfKind::IPv6)?;
        let info = service_info(&advert)?;
        let own_fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        let discovery = Self {
            daemon,
            generation,
            own_fullname,
            advert,
            events,
        };
        discovery.browse()?;
        tracing::info!(generation, "discovery mDNS aktif");
        Ok(discovery)
    }

    fn browse(&self) -> Result<(), mdns_sd::Error> {
        let rx = self.daemon.browse(MDNS_SERVICE_TYPE)?;
        tokio::spawn(forward(
            rx,
            self.generation,
            self.own_fullname.clone(),
            self.events.clone(),
        ));
        Ok(())
    }

    pub fn requery(&self) -> Result<(), mdns_sd::Error> {
        let _ = self.daemon.stop_browse(MDNS_SERVICE_TYPE);
        self.browse()
    }

    pub fn update(&mut self, advert: &LocalAdvert) -> Result<(), mdns_sd::Error> {
        if *advert == self.advert {
            return Ok(());
        }
        self.daemon.register(service_info(advert)?)?;
        self.advert = advert.clone();
        Ok(())
    }

    pub fn verify(&self, fullname: &str) {
        if let Err(e) = self
            .daemon
            .verify(fullname.to_string(), Duration::from_secs(3))
        {
            tracing::debug!("verify mDNS gagal: {e}");
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn shutdown(self) {
        let _ = self.daemon.unregister(&self.own_fullname);
        let _ = self.daemon.shutdown();
        tracing::info!(generation = self.generation, "discovery mDNS dimatikan");
    }
}

fn instance_name(id: &Uuid) -> String {
    format!("tautra-{}", id.simple())
}

fn service_info(advert: &LocalAdvert) -> Result<ServiceInfo, mdns_sd::Error> {
    let host = format!("{}.local.", instance_name(&advert.device_id));
    let props: Vec<(&str, String)> = vec![
        (txt::DEVICE_ID, advert.device_id.to_string()),
        (txt::NAME, advert.device_name.clone()),
        (txt::PROTOCOL, tautra_proto::PROTOCOL_VERSION.to_string()),
        (txt::BUILD, advert.build.to_string()),
        (
            txt::PAIRING,
            if advert.pairing_open { "1" } else { "0" }.to_string(),
        ),
        (
            txt::WIRELESS,
            advert
                .wireless
                .iter()
                .map(|ip| ip.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ),
    ];
    Ok(ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance_name(&advert.device_id),
        &host,
        "",
        advert.port,
        &props[..],
    )?
    .enable_addr_auto())
}

async fn forward(
    rx: mdns_sd::Receiver<ServiceEvent>,
    generation: u64,
    own_fullname: String,
    tx: mpsc::Sender<TaggedEvent>,
) {
    while let Ok(event) = rx.recv_async().await {
        let mapped = match event {
            ServiceEvent::ServiceResolved(svc) => {
                if svc.get_fullname().eq_ignore_ascii_case(&own_fullname) {
                    continue;
                }
                match parse_advert(&svc) {
                    Some(advert) => DiscoveryEvent::Resolved(advert),
                    None => {
                        tracing::debug!(fullname = svc.get_fullname(), "iklan mDNS tidak lengkap");
                        continue;
                    }
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => DiscoveryEvent::Removed { fullname },
            _ => continue,
        };
        if tx.send((generation, mapped)).await.is_err() {
            break;
        }
    }
}

fn parse_advert(svc: &ResolvedService) -> Option<PeerAdvert> {
    let props: HashMap<String, String> = svc.get_properties().clone().into_property_map_str();
    let device_id = Uuid::parse_str(props.get(txt::DEVICE_ID)?).ok()?;
    let mut addrs: Vec<Ipv4Addr> = svc
        .get_addresses_v4()
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect();
    if addrs.is_empty() || svc.get_port() == 0 {
        return None;
    }
    addrs.sort_by_key(|ip| (ip.is_link_local(), *ip));
    Some(PeerAdvert {
        fullname: svc.get_fullname().to_string(),
        device_id,
        device_name: props
            .get(txt::NAME)
            .cloned()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Tautra".to_string()),
        addrs,
        port: svc.get_port(),
        protocol_version: props
            .get(txt::PROTOCOL)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        pairing_open: props.get(txt::PAIRING).map(|v| v == "1").unwrap_or(false),
        wireless: props
            .get(txt::WIRELESS)
            .map(|v| v.split(',').filter_map(|ip| ip.trim().parse().ok()).collect())
            .unwrap_or_default(),
        build: props
            .get(txt::BUILD)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    })
}
