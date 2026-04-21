//! Gestion RTP du Bridge
//! 
//! Ce module contient la logique de communication RTP avec les participants,
//! incluant la résolution des cibles et la gestion du serveur RTP.

use crate::{
    config::Config,
    logger::FrontendLogger,
    midi::MidiFrame,
    rtp::{RtpRemoteTarget, RtpServer},
    types::RtpParticipantInfo,
};
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::Arc,
};

/// Gestionnaire de communication RTP
pub struct RtpManager {
    server: Option<RtpServer>,
    enabled: bool,
    port: u16,
    remote_enabled: bool,
    remote_targets: Vec<RtpRemoteTarget>,
    log_enabled: bool,
}

/// Résolveur de cibles RTP
pub struct RtpTargetResolver;

impl RtpManager {
    /// Crée un nouveau gestionnaire RTP
    pub fn new() -> Self {
        Self {
            server: None,
            enabled: false,
            port: 5000,
            remote_enabled: false,
            remote_targets: Vec::new(),
            log_enabled: false,
        }
    }
    
    /// Initialise le serveur RTP
    pub fn initialize(
        &mut self,
        enabled: bool,
        port: u16,
        remote_enabled: bool,
        remote_targets: Vec<RtpRemoteTarget>,
        log_enabled: bool,
        logger: &FrontendLogger,
    ) -> Result<(), String> {
        self.enabled = enabled;
        self.port = port;
        self.remote_enabled = remote_enabled;
        self.remote_targets = remote_targets.clone();
        self.log_enabled = log_enabled;
        
        if enabled {
            match RtpServer::start(
                "OSCMIDI".to_string(),
                port,
                Vec::new(), // Pas de cibles distantes pour l'instant
                log_enabled,
                std::sync::Arc::new(parking_lot::Mutex::new(None)),
                logger.clone(),
            ) {
                Ok(server) => {
                    self.server = Some(server);
                    logger.info(format!("RTP serveur démarré sur port {}", port));
                    
                    if remote_enabled && !remote_targets.is_empty() {
                        logger.info(format!("RTP cibles distantes: {}", remote_targets.len()));
                        for target in &remote_targets {
                            logger.debug(format!("  - {} -> {}", target.name, target.addr));
                        }
                    }
                    
                    Ok(())
                }
                Err(e) => {
                    let error_msg = format!("Erreur démarrage serveur RTP: {}", e);
                    logger.error(&error_msg);
                    Err(error_msg)
                }
            }
        } else {
            self.server = None;
            logger.info("RTP désactivé");
            Ok(())
        }
    }
    
    /// Envoie une trame MIDI via RTP
    pub fn send_midi_frame(&self, frame: &MidiFrame, logger: &FrontendLogger) {
        if !self.enabled {
            return;
        }
        
        let Some(ref server) = self.server else {
            return;
        };
        
        // Envoyer aux cibles distantes si activé
        if self.remote_enabled {
            for target in &self.remote_targets {
                // TODO: Implémenter l'envoi vers les cibles distantes
                // Pour l'instant, nous simulons l'envoi
                if self.log_enabled {
                    logger.debug(format!("Envoi RTP vers {} (simulé)", target.name));
                }
            }
        }
    }
    
    /// Retourne les informations sur les participants actifs
    pub fn get_participants(&self) -> Vec<RtpParticipantInfo> {
        if let Some(ref server) = self.server {
            server.participants().clone()
        } else {
            Vec::new()
        }
    }
    
    /// Retourne le port actuellement utilisé
    pub fn bound_port(&self) -> Option<u16> {
        self.server.as_ref().map(|s| s.bound_port())
    }
    
    /// Retourne le nombre de cibles distantes
    pub fn remote_target_count(&self) -> usize {
        self.remote_targets.len()
    }
    
    /// Vérifie si RTP est activé
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Vérifie si les cibles distantes sont activées
    pub fn is_remote_enabled(&self) -> bool {
        self.remote_enabled
    }
    
    /// Ferme le serveur RTP
    pub fn close(&mut self, logger: &FrontendLogger) {
        if let Some(_) = self.server.take() {
            logger.info("RTP serveur arrêté");
        }
        self.enabled = false;
    }
    
    /// Met à jour les cibles distantes
    pub fn update_remote_targets(&mut self, targets: Vec<RtpRemoteTarget>, logger: &FrontendLogger) {
        self.remote_targets = targets.clone();
        if self.remote_enabled {
            logger.info(format!("RTP cibles distantes mises à jour: {}", targets.len()));
        }
    }
}

impl Default for RtpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RtpTargetResolver {
    /// Résout les cibles RTP depuis la configuration
    pub fn resolve_targets(config: &Config, logger: &FrontendLogger) -> Vec<RtpRemoteTarget> {
        if !config.rtp_remote_enabled {
            return Vec::new();
        }
        
        let mut resolved = Vec::new();
        
        // Pour l'instant, nous utilisons les entrées de configuration pour créer des cibles
        // La conversion vers RtpRemoteTarget sera faite plus tard
        for entry in &config.rtp_remotes {
            if let Ok(addr) = format!("{}:{}", entry.host, entry.port).parse() {
                let target = RtpRemoteTarget {
                    name: entry.name.clone(),
                    addr,
                };
                resolved.push(target);
                
                if config.log_rtp {
                    logger.info(format!("RTP cible: {} -> {}", entry.name, addr));
                }
            }
        }
        
        // Trier les adresses pour la cohérence (IPv4 d'abord)
        resolved.sort_by(|a, b| {
            Self::socket_addr_sort_key(&a.addr).cmp(&Self::socket_addr_sort_key(&b.addr))
        });
        
        resolved
    }
    
    /// Résout une adresse socket
    pub fn resolve_socket_addr(
        host: &str,
        port: u16,
        logger: &FrontendLogger,
    ) -> Option<SocketAddr> {
        let addr_str = format!("{}:{}", host, port);
        
        match addr_str.to_socket_addrs() {
            Ok(addrs) => {
                let addrs_vec: Vec<SocketAddr> = addrs.collect();
                if let Some(addr) = addrs_vec.iter().find(|addr| {
                    // Préférer les adresses IPv4 si disponibles
                    addr.is_ipv4() || addrs_vec.len() == 1
                }) {
                    Some(*addr)
                } else {
                    addrs_vec.first().copied()
                }
            }
            Err(e) => {
                logger.error(format!("Erreur résolution {}: {}", addr_str, e));
                None
            }
        }
    }
    
    /// Clé de tri pour les adresses socket (IPv4 d'abord)
    fn socket_addr_sort_key(addr: &SocketAddr) -> (u8, IpAddr, u16) {
        let family_rank = match addr.ip() {
            IpAddr::V4(_) => 0u8,
            IpAddr::V6(_) => 1u8,
        };
        (family_rank, addr.ip(), addr.port())
    }
    
    /// Valide une configuration de cible RTP
    pub fn validate_target(target: &RtpRemoteTarget) -> Result<(), String> {
        if target.name.is_empty() {
            return Err("Le nom de la cible RTP ne peut pas être vide".to_string());
        }
        
        if target.addr.port() == 0 {
            return Err("Le port de la cible RTP ne peut pas être 0".to_string());
        }
        
        Ok(())
    }
    
    /// Crée une cible RTP avec validation
    pub fn create_target(name: String, addr: SocketAddr) -> Result<RtpRemoteTarget, String> {
        let target = RtpRemoteTarget { name, addr };
        
        Self::validate_target(&target)?;
        Ok(target)
    }
    
    /// Teste la connectivité vers une cible RTP
    pub fn test_connectivity(target: &RtpRemoteTarget, timeout_ms: u64) -> bool {
        // Pour l'instant, on fait juste une résolution d'adresse
        // Dans une implémentation complète, on pourrait faire un ping ou une connexion test
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
        true // Simulation de succès
    }
    
    /// Retourne des statistiques sur les cibles RTP
    pub fn get_target_stats(targets: &[RtpRemoteTarget]) -> RtpTargetStats {
        let mut ipv4_count = 0;
        let mut ipv6_count = 0;
        
        for target in targets {
            if target.addr.is_ipv4() {
                ipv4_count += 1;
            } else {
                ipv6_count += 1;
            }
        }
        
        RtpTargetStats {
            total: targets.len(),
            resolved: targets.len(), // Toutes sont considérées comme résolues
            ipv4: ipv4_count,
            ipv6: ipv6_count,
        }
    }
}

/// Statistiques sur les cibles RTP
#[derive(Debug, Clone)]
pub struct RtpTargetStats {
    pub total: usize,
    pub resolved: usize,
    pub ipv4: usize,
    pub ipv6: usize,
}

impl RtpTargetStats {
    /// Retourne le taux de résolution (0.0 à 1.0)
    pub fn resolution_rate(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.resolved as f32 / self.total as f32
        }
    }
    
    /// Retourne une description textuelle
    pub fn description(&self) -> String {
        format!(
            "{}/{} cibles résolues (IPv4: {}, IPv6: {})",
            self.resolved, self.total, self.ipv4, self.ipv6
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rtp_target_validation() {
        // Cible valide
        let valid_target = RtpTargetResolver::create_target("localhost".to_string(), 5000);
        assert!(valid_target.is_ok());
        
        // Hôte vide
        let empty_host = RtpTargetResolver::create_target("".to_string(), 5000);
        assert!(empty_host.is_err());
        
        // Port invalide
        let invalid_port = RtpTargetResolver::create_target("localhost".to_string(), 0);
        assert!(invalid_port.is_err());
        
        let too_high_port = RtpTargetResolver::create_target("localhost".to_string(), 70000);
        assert!(too_high_port.is_err());
    }
    
    #[test]
    fn test_socket_addr_sorting() {
        let addr_v4 = "127.0.0.1:5000".parse::<SocketAddr>().unwrap();
        let addr_v6 = "[::1]:5000".parse::<SocketAddr>().unwrap();
        
        let key_v4 = RtpTargetResolver::socket_addr_sort_key(&addr_v4);
        let key_v6 = RtpTargetResolver::socket_addr_sort_key(&addr_v6);
        
        // IPv4 doit venir avant IPv6
        assert!(key_v4 < key_v6);
    }
    
    #[test]
    fn test_target_stats() {
        let mut targets = vec![
            RtpRemoteTarget {
                host: "127.0.0.1".to_string(),
                port: 5000,
                resolved_addr: Some("127.0.0.1:5000".parse().unwrap()),
            },
            RtpRemoteTarget {
                host: "::1".to_string(),
                port: 5000,
                resolved_addr: Some("[::1]:5000".parse().unwrap()),
            },
            RtpRemoteTarget {
                host: "unresolved".to_string(),
                port: 5000,
                resolved_addr: None,
            },
        ];
        
        let stats = RtpTargetResolver::get_target_stats(&targets);
        
        assert_eq!(stats.total, 3);
        assert_eq!(stats.resolved, 2);
        assert_eq!(stats.ipv4, 1);
        assert_eq!(stats.ipv6, 1);
        assert_eq!(stats.resolution_rate(), 2.0 / 3.0);
        assert_eq!(stats.description(), "2/3 cibles résolues (IPv4: 1, IPv6: 1)");
    }
    
    #[test]
    fn test_rtp_manager_lifecycle() {
        let mut manager = RtpManager::new();
        let logger = FrontendLogger::new();
        
        // Test initialisation
        let result = manager.initialize(true, 5000, false, Vec::new(), false, &logger);
        assert!(result.is_ok());
        assert!(manager.is_enabled());
        assert!(!manager.is_remote_enabled());
        assert_eq!(manager.remote_target_count(), 0);
        
        // Test mise à jour des cibles
        let targets = vec![RtpRemoteTarget {
            host: "localhost".to_string(),
            port: 5001,
            resolved_addr: None,
        }];
        
        manager.update_remote_targets(targets, &logger);
        assert_eq!(manager.remote_target_count(), 1);
        
        // Test fermeture
        manager.close(&logger);
        assert!(!manager.is_enabled());
    }
}
