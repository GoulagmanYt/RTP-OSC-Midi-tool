use mdns_sd::{ServiceDaemon, ServiceEvent};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::async_runtime::{self, JoinHandle};
use tauri::Emitter;
use tokio::sync::oneshot;
use tokio::time::interval;

use crate::{logger::background_log, types::RtpSessionInfo};

const RTP_SESSIONS_EVENT: &str = "rtp:sessions";
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(500);
const DISCOVERY_TTL: Duration = Duration::from_secs(30);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const DISCOVERY_INTERVAL_IDLE: Duration = Duration::from_secs(30);

pub fn ports_available(port: u16) -> Result<bool, String> {
    if port == u16::MAX {
        return Ok(false);
    }
    let ctrl_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    let data_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port.saturating_add(1));
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
    let data = match UdpSocket::bind(data_addr) {
        Ok(sock) => sock,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::AddrInUse {
                return Ok(false);
            } else {
                return Err(err.to_string());
            }
        }
    };
    drop(ctrl);
    drop(data);
    Ok(true)
}

pub fn discover_sessions(timeout: Duration) -> Result<Vec<RtpSessionInfo>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = mdns
        .browse("_apple-midi._udp.local.")
        .map_err(|e| e.to_string())?;

    let deadline = Instant::now() + timeout;
    let mut sessions: HashMap<String, RtpSessionInfo> = HashMap::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(200));
        let event = match receiver.recv_timeout(wait) {
            Ok(event) => event,
            Err(_) => continue,
        };

        if let ServiceEvent::ServiceResolved(info) = event {
            if info.get_addresses().is_empty() {
                continue;
            }

            let fullname = info.get_fullname();
            let name = fullname
                .strip_suffix("._apple-midi._udp.local.")
                .unwrap_or(fullname)
                .to_string();
            let host = info.get_hostname().trim_end_matches('.').to_string();
            let mut addresses: Vec<String> = info
                .get_addresses()
                .iter()
                .map(|ip| ip.to_string())
                .collect();
            addresses.sort_by(|a, b| {
                let a_v6 = a.contains(':');
                let b_v6 = b.contains(':');
                (a_v6, a).cmp(&(b_v6, b))
            });

            let key = format!("{}:{}", fullname, info.get_port());
            sessions
                .entry(key)
                .and_modify(|existing| {
                    for addr in &addresses {
                        if !existing.addresses.contains(addr) {
                            existing.addresses.push(addr.clone());
                        }
                    }
                })
                .or_insert_with(|| RtpSessionInfo {
                    name: if name.is_empty() { host.clone() } else { name },
                    host,
                    port: info.get_port(),
                    addresses,
                });
        }
    }

    let _ = mdns.shutdown();

    let mut list: Vec<RtpSessionInfo> = sessions.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

#[derive(Default, Debug)]
struct RtpDiscoveryCache {
    sessions: Vec<RtpSessionInfo>,
    updated_at: Option<Instant>,
}

impl RtpDiscoveryCache {
    fn is_stale(&self) -> bool {
        self.updated_at
            .map(|ts| ts.elapsed() >= DISCOVERY_TTL)
            .unwrap_or(true)
    }

    fn update(&mut self, sessions: Vec<RtpSessionInfo>) -> bool {
        let changed = self.sessions != sessions;
        self.sessions = sessions;
        self.updated_at = Some(Instant::now());
        changed
    }
}

#[derive(Debug)]
pub struct RtpDiscoveryManager {
    cache: Arc<Mutex<RtpDiscoveryCache>>,
    task: Mutex<Option<JoinHandle<()>>>,
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl Clone for RtpDiscoveryManager {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            task: Mutex::new(None),
            stop_tx: Mutex::new(None),
        }
    }
}

impl RtpDiscoveryManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(RtpDiscoveryCache::default())),
            task: Mutex::new(None),
            stop_tx: Mutex::new(None),
        }
    }

    pub fn cached(&self) -> Vec<RtpSessionInfo> {
        self.cache.lock().sessions.clone()
    }

    pub fn start(&self, app_handle: tauri::AppHandle) {
        let mut task_guard = self.task.lock();
        if task_guard.is_some() {
            return;
        }
        let (stop_tx, mut stop_rx) = oneshot::channel();
        *self.stop_tx.lock() = Some(stop_tx);
        let cache = self.cache.clone();
        *task_guard = Some(async_runtime::spawn(async move {
            let initial = cache.lock().sessions.clone();
            emit_sessions(&app_handle, &initial);
            let mut ticker = interval(DISCOVERY_INTERVAL);
            let mut idle_streak = 0u32;
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = ticker.tick() => {
                        let should_refresh = { cache.lock().is_stale() };
                        if !should_refresh { continue; }

                        let handle = async_runtime::spawn_blocking(|| discover_sessions(DISCOVERY_TIMEOUT));
                        match handle.await {
                            Ok(Ok(sessions)) => {
                                let changed = { cache.lock().update(sessions.clone()) };
                                if changed {
                                    idle_streak = 0;
                                    emit_sessions(&app_handle, &sessions);
                                } else {
                                    idle_streak += 1;
                                }
                                let next = if idle_streak > 3 {
                                    DISCOVERY_INTERVAL_IDLE
                                } else {
                                    DISCOVERY_INTERVAL
                                };
                                ticker = interval(next);
                                ticker.tick().await;
                            }
                            Ok(Err(err)) => background_log("warn", format!("RTP discovery: {err}")),
                            Err(err) => background_log("warn", format!("RTP discovery task: {err}")),
                        }
                    }
                }
            }
        }));
    }

    /// Arrête la tâche de découverte en arrière-plan.
    ///
    /// FIX : l'ancienne implémentation utilisait `async_runtime::block_on(handle)`
    /// pour attendre la fin de la tâche. Appeler `block_on` depuis un contexte
    /// tokio (ex. un command handler Tauri async) provoque une panique
    /// "Cannot start a runtime from within a runtime".
    ///
    /// Nouveau comportement : on envoie le signal d'arrêt puis on `abort()` la
    /// tâche. La tâche se terminera proprement sur la prochaine itération du
    /// select (via stop_rx), ou sera annulée immédiatement par abort().
    /// Pas besoin d'attendre la jointure depuis un contexte sync.
    pub fn stop(&self) {
        if let Some(stop_tx) = self.stop_tx.lock().take() {
            // Signal gracieux : la boucle interne s'arrête sur la prochaine
            // itération du select!.
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.task.lock().take() {
            // abort() annule la tâche sans bloquer le thread courant.
            // Sûr à appeler depuis n'importe quel contexte (sync ou async).
            handle.abort();
        }
    }

    /// Découvre les sessions RTP immédiatement et met à jour le cache.
    ///
    /// FIX : l'ancienne implémentation utilisait `async_runtime::block_on`
    /// autour d'un `spawn_blocking`, ce qui panique si appelé depuis un
    /// context tokio.
    ///
    /// Nouveau comportement : `tokio::task::block_in_place` exécute le code
    /// bloquant directement sur le thread courant après l'avoir sorti du pool
    /// async — valide depuis un contexte tokio multi-thread (utilisé par Tauri).
    pub fn refresh_now(
        &self,
        app_handle: &tauri::AppHandle,
        force: bool,
    ) -> Result<Vec<RtpSessionInfo>, String> {
        if !force {
            let cache = self.cache.lock();
            if !cache.is_stale() {
                let sessions = cache.sessions.clone();
                drop(cache);
                emit_sessions(app_handle, &sessions);
                return Ok(sessions);
            }
        }

        // block_in_place est sans danger depuis un runtime tokio multi-thread :
        // il cède temporairement le thread au code bloquant sans bloquer
        // l'executor, contrairement à block_on qui paniquerait ici.
        let sessions = tokio::task::block_in_place(|| discover_sessions(DISCOVERY_TIMEOUT))?;

        let _changed = self.cache.lock().update(sessions.clone());
        emit_sessions(app_handle, &sessions);
        Ok(sessions)
    }
}

impl Default for RtpDiscoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

fn emit_sessions(app_handle: &tauri::AppHandle, sessions: &[RtpSessionInfo]) {
    let _ = app_handle.emit(RTP_SESSIONS_EVENT, sessions);
}
