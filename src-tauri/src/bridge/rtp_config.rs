use crate::{config::Config, logger::FrontendLogger, rtp::RtpRemoteTarget};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
};

pub(super) fn resolve_remote_targets(
    config: &Config,
    logger: &FrontendLogger,
) -> Vec<RtpRemoteTarget> {
    if !config.rtp.remote_enabled {
        return Vec::new();
    }

    let entries = config.rtp.remotes.clone();

    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let entry_name = entry.name.trim().to_string();
        if !entry.auto_connect {
            continue;
        }
        let host = entry.host.trim();
        if host.is_empty() {
            logger.warn(format!("RTP-MIDI remote host is empty for {}", entry_name));
            continue;
        }
        if entry.port == 0 {
            logger.warn(format!("RTP-MIDI remote port is 0 for {}", entry_name));
            continue;
        }

        let resolved_addr = resolve_remote_socket_addr(host, entry.port, logger);
        let Some(addr) = resolved_addr else {
            continue;
        };
        if !seen.insert(addr) {
            continue;
        }
        resolved.push(RtpRemoteTarget {
            name: if entry_name.is_empty() {
                host.to_string()
            } else {
                entry_name
            },
            addr,
        });
    }
    resolved
}

fn resolve_remote_socket_addr(
    host: &str,
    port: u16,
    logger: &FrontendLogger,
) -> Option<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(SocketAddr::new(ip, port));
    }

    let addr = format!("{host}:{port}");
    match addr.to_socket_addrs() {
        Ok(iter) => {
            let mut addresses: Vec<SocketAddr> = iter.collect();
            addresses.sort_by_key(socket_addr_sort_key);
            addresses.dedup();
            let selected = addresses.first().copied();
            if selected.is_none() {
                logger.warn(format!("RTP-MIDI remote host unresolved: {host}"));
            }
            selected
        }
        Err(err) => {
            logger.warn(format!("RTP-MIDI remote host invalid: {host} ({err})"));
            None
        }
    }
}

fn socket_addr_sort_key(addr: &SocketAddr) -> (u8, IpAddr, u16) {
    let family_rank = match addr.ip() {
        IpAddr::V4(_) => 0u8,
        IpAddr::V6(_) => 1u8,
    };
    (family_rank, addr.ip(), addr.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_addr_sort_prefers_ipv4_before_ipv6() {
        let ipv4: SocketAddr = "127.0.0.1:5004".parse().expect("ipv4");
        let ipv6: SocketAddr = "[::1]:5004".parse().expect("ipv6");
        assert!(socket_addr_sort_key(&ipv4) < socket_addr_sort_key(&ipv6));
    }
}
