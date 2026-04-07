use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Device status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_type: String,
    pub name: String,
    pub ip: String,
    pub status: DeviceStatus,
    pub last_heartbeat: u64,
    pub properties: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DeviceStatus {
    Online,
    Offline,
    Unknown,
}

/// MQTT-like message (simplified protocol simulation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayMessage {
    pub topic: String,
    pub payload: Value,
    pub qos: u8,
    pub retain: bool,
}

/// Device registration request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub device_id: String,
    pub device_type: String,
    pub name: String,
    pub ip: String,
    pub capabilities: Vec<String>,
}

/// Heartbeat packet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub device_id: String,
    pub timestamp: u64,
    pub status: HashMap<String, Value>,
}

/// Gateway — central hub for IoT device management
/// Conceptual implementation: manages device registry, heartbeats, and message routing
pub struct Gateway {
    devices: Arc<RwLock<HashMap<String, DeviceInfo>>>,
    message_handlers: Arc<RwLock<HashMap<String, Vec<Box<dyn Fn(&GatewayMessage) + Send + Sync>>>>>,
    heartbeat_timeout_secs: u64,
}

impl Gateway {
    pub fn new() -> Self {
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            message_handlers: Arc::new(RwLock::new(HashMap::new())),
            heartbeat_timeout_secs: 60,
        }
    }

    /// Register a device
    pub async fn register_device(&self, reg: DeviceRegistration) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let device = DeviceInfo {
            device_id: reg.device_id.clone(),
            device_type: reg.device_type,
            name: reg.name,
            ip: reg.ip,
            status: DeviceStatus::Online,
            last_heartbeat: now,
            properties: HashMap::new(),
        };

        let mut devices = self.devices.write().await;
        println!("  Gateway: device registered: {} ({})", device.name, device.device_id);
        devices.insert(reg.device_id, device);
        Ok(())
    }

    /// Process a heartbeat from a device
    pub async fn process_heartbeat(&self, heartbeat: Heartbeat) -> Result<(), String> {
        let mut devices = self.devices.write().await;

        if let Some(device) = devices.get_mut(&heartbeat.device_id) {
            device.last_heartbeat = heartbeat.timestamp;
            device.status = DeviceStatus::Online;

            // Update properties from heartbeat status
            for (key, value) in heartbeat.status {
                device.properties.insert(key, value);
            }

            Ok(())
        } else {
            Err(format!("Unknown device: {}", heartbeat.device_id))
        }
    }

    /// Get all registered devices
    pub async fn list_devices(&self) -> Vec<DeviceInfo> {
        let devices = self.devices.read().await;
        devices.values().cloned().collect()
    }

    /// Get a specific device
    pub async fn get_device(&self, device_id: &str) -> Option<DeviceInfo> {
        let devices = self.devices.read().await;
        devices.get(device_id).cloned()
    }

    /// Check for offline devices (heartbeat timeout)
    pub async fn check_device_health(&self) -> Vec<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut offline_devices = Vec::new();
        let mut devices = self.devices.write().await;

        for (id, device) in devices.iter_mut() {
            if now - device.last_heartbeat > self.heartbeat_timeout_secs
                && device.status == DeviceStatus::Online
            {
                device.status = DeviceStatus::Offline;
                offline_devices.push(id.clone());
                println!("  Gateway: device offline: {} ({})", device.name, id);
            }
        }

        offline_devices
    }

    /// Publish a message (simulated MQTT publish)
    pub async fn publish(&self, message: GatewayMessage) {
        println!(
            "  Gateway: publish topic={} payload={}",
            message.topic,
            serde_json::to_string(&message.payload).unwrap_or_default()
        );

        // Route to matching topic handlers
        let handlers = self.message_handlers.read().await;
        for (pattern, handler_list) in handlers.iter() {
            if topic_matches(pattern, &message.topic) {
                for handler in handler_list {
                    handler(&message);
                }
            }
        }
    }

    /// Simulate mDNS device discovery (UDP broadcast concept)
    pub async fn discover_devices(&self) -> Vec<DiscoveredDevice> {
        println!("  Gateway: initiating mDNS discovery (simulated)...");

        // In a real implementation, this would send UDP broadcast packets
        // and listen for responses. Here we simulate the concept.
        let discovered = vec![
            DiscoveredDevice {
                name: "Smart Light (Living Room)".into(),
                service_type: "_iot._tcp.local".into(),
                ip: "192.168.1.100".into(),
                port: 8080,
            },
            DiscoveredDevice {
                name: "Temperature Sensor".into(),
                service_type: "_iot._tcp.local".into(),
                ip: "192.168.1.101".into(),
                port: 8080,
            },
        ];

        println!("  Gateway: discovered {} devices", discovered.len());
        discovered
    }

    /// Status report as JSON
    pub async fn status_report(&self) -> Value {
        let devices = self.list_devices().await;
        let online = devices.iter().filter(|d| d.status == DeviceStatus::Online).count();
        let offline = devices.iter().filter(|d| d.status == DeviceStatus::Offline).count();

        json!({
            "total_devices": devices.len(),
            "online": online,
            "offline": offline,
            "devices": devices.iter().map(|d| json!({
                "id": d.device_id,
                "name": d.name,
                "type": d.device_type,
                "status": format!("{:?}", d.status),
                "ip": d.ip,
            })).collect::<Vec<_>>()
        })
    }
}

impl Default for Gateway {
    fn default() -> Self {
        Self::new()
    }
}

/// Discovered device via mDNS
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub name: String,
    pub service_type: String,
    pub ip: String,
    pub port: u16,
}

/// Simple MQTT topic matching (supports + and # wildcards)
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let topic_parts: Vec<&str> = topic.split('/').collect();

    let mut pi = 0;
    let mut ti = 0;

    while pi < pattern_parts.len() && ti < topic_parts.len() {
        match pattern_parts[pi] {
            "#" => return true, // multi-level wildcard
            "+" => {
                // single-level wildcard
                pi += 1;
                ti += 1;
            }
            exact => {
                if exact != topic_parts[ti] {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }

    pi == pattern_parts.len() && ti == topic_parts.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_matches() {
        // Exact match
        assert!(topic_matches("home/light/1", "home/light/1"));
        assert!(!topic_matches("home/light/1", "home/light/2"));

        // Single-level wildcard (+)
        assert!(topic_matches("home/+/status", "home/light/status"));
        assert!(topic_matches("home/+/status", "home/ac/status"));
        assert!(!topic_matches("home/+/status", "home/light/1/status"));

        // Multi-level wildcard (#)
        assert!(topic_matches("home/#", "home/light/1"));
        assert!(topic_matches("home/#", "home/light/1/status"));
        // Note: "home/#" does not match "home" because # must match at least one level
        assert!(!topic_matches("home/#", "home"));

        // No match
        assert!(!topic_matches("office/light", "home/light"));
    }
}
