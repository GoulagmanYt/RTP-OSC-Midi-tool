use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

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
    status: RtpAdvertisementStatus,
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
                    status: RtpAdvertisementStatus {
                        warning: Some(warning),
                        ..RtpAdvertisementStatus::default()
                    },
                };
            }
        };

        let addresses = select_advertised_addresses(&interfaces);
        if addresses.is_empty() {
            let warning = "RTP-MIDI mDNS has no usable local IPv4 address to advertise".to_string();
            logger.warn(warning.clone());
            return Self {
                daemon: None,
                status: RtpAdvertisementStatus {
                    warning: Some(warning),
                    ..RtpAdvertisementStatus::default()
                },
            };
        }

        let host_name = advertised_host_name(session_name, "");
        let address_text = addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        match register_mdns_service(session_name, &host_name, addresses, port) {
            Ok(daemon) => {
                logger.info(format!(
                    "RTP-MIDI mDNS advertised as {host_name} on {}",
                    address_text.join(", ")
                ));
                Self {
                    daemon: Some(daemon),
                    status: RtpAdvertisementStatus {
                        advertised_host: Some(host_name),
                        advertised_addresses: address_text,
                        warning: None,
                    },
                }
            }
            Err(err) => {
                let warning = format!("RTP-MIDI mDNS advertisement failed: {err}");
                logger.warn(warning.clone());
                Self {
                    daemon: None,
                    status: RtpAdvertisementStatus {
                        advertised_host: Some(host_name),
                        advertised_addresses: address_text,
                        warning: Some(warning),
                    },
                }
            }
        }
    }

    pub fn status(&self) -> RtpAdvertisementStatus {
        self.status.clone()
    }

    pub fn shutdown(self) {
        if let Some(daemon) = self.daemon {
            let _ = daemon.shutdown();
        }
    }
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
) -> Result<ServiceDaemon, String> {
    let mdns = ServiceDaemon::new().map_err(|err| err.to_string())?;
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
    mdns.register(service).map_err(|err| err.to_string())?;
    Ok(mdns)
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
