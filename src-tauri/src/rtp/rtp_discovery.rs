use mdns_sd::{ServiceDaemon, ServiceEvent};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::Duration;
use tauri::async_runtime::{self, JoinHandle};
use tauri::Emitter;
use tokio::sync::oneshot;

use crate::{logger::background_log, types::RtpSessionInfo};

const RTP_SESSIONS_EVENT: &str = "rtp:sessions";

pub fn ports_available(port: u16) -> Result<bool, String> {
    if port == u16::MAX {
        return Ok(false);
    }
    for attempt in 0..32 {
        let ctrl_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        let ctrl = match UdpSocket::bind(ctrl_addr) {
            Ok(sock) => sock,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::AddrInUse {
                    return Ok(false);
                } else {
                    return Err(err.to_string());
                }
            }
        };
        let Some(data_port) = ctrl
            .local_addr()
            .map_err(|err| err.to_string())?
            .port()
            .checked_add(1)
        else {
            if port == 0 {
                continue;
            }
            return Ok(false);
        };
        let data_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, data_port);
        let data = match UdpSocket::bind(data_addr) {
            Ok(sock) => sock,
            Err(err) => {
                if port == 0 && attempt < 31 && err.kind() == std::io::ErrorKind::AddrInUse {
                    continue;
                }
                if err.kind() == std::io::ErrorKind::AddrInUse {
                    return Ok(false);
                } else {
                    return Err(err.to_string());
                }
            }
        };
        drop(ctrl);
        drop(data);
        return Ok(true);
    }
    Ok(false)
}

#[derive(Clone)]
struct LocalSession {
    name: String,
    port: u16,
    addresses: Vec<String>,
}

#[derive(Clone)]
pub struct RtpDiscoveryManager {
    daemon: Arc<Mutex<Option<ServiceDaemon>>>,
    sessions: Arc<Mutex<HashMap<String, RtpSessionInfo>>>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    stop_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    local_filter: Arc<Mutex<Option<LocalSession>>>,
}

impl RtpDiscoveryManager {
    pub fn new() -> Self {
        Self {
            daemon: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            task: Arc::new(Mutex::new(None)),
            stop_tx: Arc::new(Mutex::new(None)),
            local_filter: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_local_session(&self, name: String, port: u16) {
        let mut addresses = local_ip_address::list_afinet_netifas()
            .unwrap_or_default()
            .into_iter()
            .map(|(_, ip)| ip.to_string())
            .collect::<Vec<_>>();
        addresses.extend(["127.0.0.1".into(), "::1".into()]);
        *self.local_filter.lock() = Some(LocalSession {
            name,
            port,
            addresses,
        });
    }

    pub fn cached(&self) -> Vec<RtpSessionInfo> {
        let map = self.sessions.lock();
        filter_and_sort_sessions(&map, &self.local_filter.lock())
    }

    pub fn start(&self, app_handle: tauri::AppHandle) {
        let mut task_guard = self.task.lock();
        if task_guard.is_some() {
            return;
        }
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(err) => {
                background_log("warn", format!("RTP discovery daemon error: {err}"));
                return;
            }
        };
        let receiver = match daemon.browse("_apple-midi._udp.local.") {
            Ok(rx) => rx,
            Err(err) => {
                background_log("warn", format!("RTP discovery browse error: {err}"));
                let _ = daemon.shutdown();
                return;
            }
        };
        *self.daemon.lock() = Some(daemon);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        *self.stop_tx.lock() = Some(stop_tx);
        let sessions_map = self.sessions.clone();
        let local_filter = self.local_filter.clone();

        *task_guard = Some(async_runtime::spawn_blocking(move || loop {
            if !matches!(stop_rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if info.get_addresses().is_empty() {
                        continue;
                    }
                    let mut map = sessions_map.lock();
                    let changed = update_resolved_service(
                        &mut map,
                        info.get_fullname(),
                        session_from_info(&info),
                    );
                    if changed {
                        let list = filter_and_sort_sessions(&map, &local_filter.lock());
                        emit_sessions(&app_handle, &list);
                    }
                }
                Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                    let mut map = sessions_map.lock();
                    let before_len = map.len();
                    map.remove(&fullname);
                    if map.len() != before_len {
                        let list = filter_and_sort_sessions(&map, &local_filter.lock());
                        emit_sessions(&app_handle, &list);
                    }
                }
                _ => {}
            }
        }));
    }

    pub fn stop(&self) {
        let mut task = self.task.lock();
        if let Some(stop_tx) = self.stop_tx.lock().take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = task.take() {
            // spawn_blocking cannot be aborted after starting. Wait for the
            // bounded receive loop before clearing the cache or restarting.
            let _ = crate::tauri::utils::safe_block_on(handle);
        }
        if let Some(daemon) = self.daemon.lock().take() {
            let _ = daemon.shutdown();
        }
        self.sessions.lock().clear();
    }

    pub fn refresh_now(
        &self,
        app_handle: &tauri::AppHandle,
        _force: bool,
    ) -> Result<Vec<RtpSessionInfo>, String> {
        let list = self.cached();
        emit_sessions(app_handle, &list);
        Ok(list)
    }
}

impl Default for RtpDiscoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

fn session_from_info(info: &mdns_sd::ResolvedService) -> RtpSessionInfo {
    let fullname = info.get_fullname();
    let name = fullname
        .strip_suffix("._apple-midi._udp.local.")
        .unwrap_or(fullname)
        .to_string();
    let host = info.get_hostname().trim_end_matches('.').to_string();
    let mut addresses: Vec<String> = info
        .get_addresses()
        .iter()
        .map(ToString::to_string)
        .collect();
    addresses.sort_by(|a, b| (a.contains(':'), a).cmp(&(b.contains(':'), b)));
    RtpSessionInfo {
        name: if name.is_empty() { host.clone() } else { name },
        host,
        port: info.get_port(),
        addresses,
    }
}

fn update_resolved_service(
    map: &mut HashMap<String, RtpSessionInfo>,
    fullname: &str,
    session: RtpSessionInfo,
) -> bool {
    let changed = map.get(fullname).is_none_or(|previous| {
        previous.host != session.host
            || previous.port != session.port
            || previous.addresses != session.addresses
            || previous.name != session.name
    });
    map.insert(fullname.to_string(), session);
    changed
}

fn filter_and_sort_sessions(
    map: &HashMap<String, RtpSessionInfo>,
    filter: &Option<LocalSession>,
) -> Vec<RtpSessionInfo> {
    let mut list: Vec<RtpSessionInfo> = map
        .values()
        .filter(|s| {
            if let Some(local) = filter.as_ref() {
                if s.port == local.port
                    && s.name.eq_ignore_ascii_case(&local.name)
                    && s.addresses
                        .iter()
                        .any(|address| local.addresses.contains(address))
                {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

fn emit_sessions(app_handle: &tauri::AppHandle, sessions: &[RtpSessionInfo]) {
    let _ = app_handle.emit(RTP_SESSIONS_EVENT, sessions);
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[test]
    fn same_name_and_port_do_not_hide_a_remote_machine() {
        let map = HashMap::from([(
            "Peer._apple-midi._udp.local.".into(),
            RtpSessionInfo {
                name: "OSCMidi".into(),
                host: "remote.local".into(),
                port: 5004,
                addresses: vec!["192.0.2.20".into()],
            },
        )]);
        assert_eq!(
            filter_and_sort_sessions(
                &map,
                &Some(LocalSession {
                    name: "OSCMidi".into(),
                    port: 5004,
                    addresses: vec!["192.0.2.10".into()]
                })
            )
            .len(),
            1
        );
    }

    #[test]
    fn resolved_service_replaces_obsolete_addresses_and_ports() {
        let mut map = HashMap::new();
        let make = |host: &str, port, address: &str| RtpSessionInfo {
            name: "Peer".into(),
            host: host.into(),
            port,
            addresses: vec![address.into()],
        };
        assert!(update_resolved_service(
            &mut map,
            "Peer._apple-midi._udp.local.",
            make("old.local", 5004, "192.0.2.10")
        ));
        assert!(update_resolved_service(
            &mut map,
            "Peer._apple-midi._udp.local.",
            make("new.local", 5006, "192.0.2.20")
        ));
        assert_eq!(map.len(), 1);
        let session = map.values().next().unwrap();
        assert_eq!(session.host, "new.local");
        assert_eq!(session.port, 5006);
        assert_eq!(session.addresses, ["192.0.2.20"]);
    }

    #[test]
    fn ephemeral_port_checks_use_an_adjacent_ephemeral_pair() {
        assert_eq!(ports_available(0), Ok(true));
        assert_eq!(ports_available(u16::MAX), Ok(false));
        let occupied = UdpSocket::bind("0.0.0.0:0").unwrap();
        assert_eq!(
            ports_available(occupied.local_addr().unwrap().port()),
            Ok(false)
        );
    }
}
