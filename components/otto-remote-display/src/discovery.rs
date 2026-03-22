use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::time::Duration;
use tracing::{debug, info};

const AIRPLAY_SERVICE: &str = "_airplay._tcp.local.";

#[derive(Debug, Clone)]
pub struct AirPlayDevice {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub model: String,
    pub device_id: String,
    pub features: String,
}

/// Browse the local network for AirPlay receivers via mDNS.
pub fn browse_airplay(timeout: Duration) -> Result<Vec<AirPlayDevice>> {
    let mdns = ServiceDaemon::new().map_err(|e| anyhow::anyhow!("mDNS init failed: {}", e))?;
    let receiver = mdns
        .browse(AIRPLAY_SERVICE)
        .map_err(|e| anyhow::anyhow!("mDNS browse failed: {}", e))?;

    let mut devices = Vec::new();
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let props = info.get_properties();

                let model = props
                    .get_property_val_str("model")
                    .unwrap_or_default()
                    .to_string();
                let device_id = props
                    .get_property_val_str("deviceid")
                    .unwrap_or_default()
                    .to_string();
                let features = props
                    .get_property_val_str("features")
                    .unwrap_or_default()
                    .to_string();

                let addr = info.get_addresses().iter().next().copied();
                if let Some(addr) = addr {
                    let raw_name = info.get_fullname();
                    let name = raw_name
                        .split("._airplay")
                        .next()
                        .unwrap_or(raw_name)
                        .to_string();

                    // Skip non-Apple TV devices (Macs, HomePods, etc.) unless explicitly targeted
                    let device = AirPlayDevice {
                        name,
                        ip: addr.to_string(),
                        port: info.get_port(),
                        model,
                        device_id,
                        features,
                    };
                    info!(
                        "Found: {} ({}) at {}:{}",
                        device.name, device.model, device.ip, device.port
                    );
                    devices.push(device);
                }
            }
            Ok(event) => {
                debug!("mDNS event: {:?}", event);
            }
            Err(_) => {
                // recv timeout, continue scanning
            }
        }
    }

    mdns.shutdown().ok();
    Ok(devices)
}
