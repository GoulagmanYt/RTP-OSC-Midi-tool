use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::logger::FrontendLogger;

const APPLE_MIDI_SERVICE_TYPE: &str = "_apple-midi._udp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalInterfaceAddress {
    pub(crate) name: String,
    pub(crate) ip: IpAddr,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RtpAdvertisementStatus {
    pub advertised_host: Option<String>,
    pub advertised_addresses: Vec<String>,
    pub warning: Option<String>,
}

pub struct RtpMdnsAdvertisement {
    daemon: Option<ServiceDaemon>,
    status: Arc<Mutex<RtpAdvertisementStatus>>,
    monitor_stop: Option<mpsc::Sender<()>>,
    monitor: Option<JoinHandle<()>>,
}

struct RtpMdnsMonitorConfig {
    session_name: String,
    host_name: String,
    service_fullname: String,
    port: u16,
}

impl RtpMdnsAdvertisement {
    pub fn start(session_name: &str, port: u16, logger: &FrontendLogger) -> Self {
        let interfaces = match collect_local_interface_addresses() {
            Ok(interfaces) => interfaces,
            Err(err) => {
                let warning = format!("RTP-MIDI mDNS address scan failed: {err}");
                logger.warn(warning.clone());
                return Self {
                    daemon: None,
                    status: Arc::new(Mutex::new(RtpAdvertisementStatus {
                        warning: Some(warning),
                        ..RtpAdvertisementStatus::default()
                    })),
                    monitor_stop: None,
                    monitor: None,
                };
            }
        };

        let addresses = select_advertised_addresses(&interfaces);
        if addresses.is_empty() {
            let warning = "RTP-MIDI mDNS has no usable local IPv4 address to advertise".to_string();
            logger.warn(warning.clone());
            return Self {
                daemon: None,
                status: Arc::new(Mutex::new(RtpAdvertisementStatus {
                    warning: Some(warning),
                    ..RtpAdvertisementStatus::default()
                })),
                monitor_stop: None,
                monitor: None,
            };
        }

        let host_name = advertised_host_name(session_name, "");
        let address_text = addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        match register_mdns_service(session_name, &host_name, addresses, port) {
            Ok((daemon, service_fullname)) => {
                logger.info(format!(
                    "RTP-MIDI mDNS advertised as {host_name} on {}",
                    address_text.join(", ")
                ));
                let status = Arc::new(Mutex::new(RtpAdvertisementStatus {
                    advertised_host: Some(host_name.clone()),
                    advertised_addresses: address_text,
                    warning: None,
                }));
                let (monitor_stop, monitor_rx) = mpsc::channel();
                let monitor = spawn_interface_monitor(
                    daemon.clone(),
                    RtpMdnsMonitorConfig {
                        session_name: session_name.to_string(),
                        host_name,
                        service_fullname,
                        port,
                    },
                    Arc::clone(&status),
                    monitor_rx,
                    logger.clone(),
                );
                Self {
                    daemon: Some(daemon),
                    status,
                    monitor_stop: Some(monitor_stop),
                    monitor,
                }
            }
            Err(err) => {
                let warning = format!("RTP-MIDI mDNS advertisement failed: {err}");
                logger.warn(warning.clone());
                Self {
                    daemon: None,
                    status: Arc::new(Mutex::new(RtpAdvertisementStatus {
                        advertised_host: Some(host_name),
                        advertised_addresses: address_text,
                        warning: Some(warning),
                    })),
                    monitor_stop: None,
                    monitor: None,
                }
            }
        }
    }

    pub fn status(&self) -> RtpAdvertisementStatus {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn shutdown(mut self) {
        if let Some(stop) = self.monitor_stop.take() {
            let _ = stop.send(());
        }
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        if let Some(daemon) = self.daemon {
            let _ = daemon.shutdown();
        }
    }
}

fn spawn_interface_monitor(
    daemon: ServiceDaemon,
    config: RtpMdnsMonitorConfig,
    status: Arc<Mutex<RtpAdvertisementStatus>>,
    stop: mpsc::Receiver<()>,
    logger: FrontendLogger,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("rtp-mdns-monitor".to_string())
        .spawn(move || loop {
            match stop.recv_timeout(Duration::from_secs(5)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let Ok(interfaces) = collect_local_interface_addresses() else {
                continue;
            };
            let addresses = select_advertised_addresses(&interfaces);
            let address_text = addresses
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let unchanged = status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .advertised_addresses
                == address_text;
            if unchanged {
                continue;
            }
            if addresses.is_empty() {
                let _ = daemon.unregister(&config.service_fullname);
                let mut current = status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                current.advertised_addresses.clear();
                current.warning =
                    Some("RTP-MIDI mDNS has no usable local IPv4 address to advertise".to_string());
                continue;
            }
            match register_mdns_service_on(
                &daemon,
                &config.session_name,
                &config.host_name,
                addresses,
                config.port,
            ) {
                Ok(_) => {
                    logger.info(format!(
                        "RTP-MIDI mDNS interfaces updated: {}",
                        address_text.join(", ")
                    ));
                    let mut current = status
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    current.advertised_addresses = address_text;
                    current.warning = None;
                }
                Err(error) => {
                    let warning = format!("RTP-MIDI mDNS interface update failed: {error}");
                    logger.warn(warning.clone());
                    status
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .warning = Some(warning);
                }
            }
        })
        .ok()
}

fn collect_local_interface_addresses() -> Result<Vec<LocalInterfaceAddress>, String> {
    let interfaces = local_ip_address::list_afinet_netifas().map_err(|err| err.to_string())?;
    Ok(interfaces
        .into_iter()
        .map(|(name, ip)| LocalInterfaceAddress { name, ip })
        .collect())
}

fn register_mdns_service(
    session_name: &str,
    host_name: &str,
    addresses: Vec<IpAddr>,
    port: u16,
) -> Result<(ServiceDaemon, String), String> {
    let mdns = ServiceDaemon::new().map_err(|err| err.to_string())?;
    let service_fullname =
        register_mdns_service_on(&mdns, session_name, host_name, addresses, port)?;
    Ok((mdns, service_fullname))
}

fn register_mdns_service_on(
    mdns: &ServiceDaemon,
    session_name: &str,
    host_name: &str,
    addresses: Vec<IpAddr>,
    port: u16,
) -> Result<String, String> {
    let properties: Option<HashMap<String, String>> = None;
    let service = ServiceInfo::new(
        APPLE_MIDI_SERVICE_TYPE,
        session_name,
        host_name,
        addresses.as_slice(),
        port,
        properties,
    )
    .map_err(|err| err.to_string())?;
    let service_fullname = service.get_fullname().to_string();
    mdns.register(service).map_err(|err| err.to_string())?;
    Ok(service_fullname)
}

pub(crate) fn select_advertised_addresses(interfaces: &[LocalInterfaceAddress]) -> Vec<IpAddr> {
    let mut addresses = interfaces
        .iter()
        .filter_map(|interface| match interface.ip {
            IpAddr::V4(ip) if is_usable_advertised_ipv4(ip) => Some(IpAddr::V4(ip)),
            _ => None,
        })
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    addresses
}

fn is_usable_advertised_ipv4(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified())
}

pub(crate) fn advertised_host_name(session_name: &str, _machine_hostname: &str) -> String {
    let mut label = session_name
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch == '-' || ch == '_' || ch == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>();

    while label.contains("--") {
        label = label.replace("--", "-");
    }
    label = label.trim_matches('-').to_string();
    if label.is_empty() {
        label = "oscmidi".to_string();
    }
    format!("{label}-rtp.local.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn iface(name: &str, ip: [u8; 4]) -> LocalInterfaceAddress {
        LocalInterfaceAddress {
            name: name.to_string(),
            ip: IpAddr::V4(Ipv4Addr::from(ip)),
        }
    }

    #[test]
    fn advertised_addresses_exclude_loopback_link_local_and_ipv6() {
        let addresses = select_advertised_addresses(&[
            iface("Loopback", [127, 0, 0, 1]),
            iface("Ethernet", [10, 86, 12, 47]),
            iface("Wi-Fi", [169, 254, 10, 20]),
            LocalInterfaceAddress {
                name: "IPv6".to_string(),
                ip: "::1".parse().expect("ipv6"),
            },
        ]);

        assert_eq!(addresses, vec![IpAddr::V4(Ipv4Addr::new(10, 86, 12, 47))]);
    }

    #[test]
    fn advertised_addresses_are_sorted_for_stable_mdns_payloads() {
        let addresses = select_advertised_addresses(&[
            iface("Wi-Fi", [10, 86, 12, 199]),
            iface("Ethernet", [10, 86, 12, 47]),
        ]);

        assert_eq!(
            addresses,
            vec![
                IpAddr::V4(Ipv4Addr::new(10, 86, 12, 47)),
                IpAddr::V4(Ipv4Addr::new(10, 86, 12, 199)),
            ]
        );
    }

    #[test]
    fn advertised_host_is_session_scoped_not_machine_hostname() {
        let host = advertised_host_name("OSCMidi", "PC_Robin-2");

        assert_eq!(host, "oscmidi-rtp.local.");
        assert_ne!(host, "PC_Robin-2.local.");
    }
}
