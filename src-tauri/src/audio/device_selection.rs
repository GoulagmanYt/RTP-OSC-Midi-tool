use cpal::{traits::HostTrait, Device, HostId};

pub(super) fn select_host(name: Option<&str>) -> Option<cpal::Host> {
    if let Some(name) = name {
        let needle = name.to_lowercase();
        let id = if needle.contains("asio") {
            Some(HostId::Asio)
        } else if needle.contains("wasapi") {
            Some(HostId::Wasapi)
        } else {
            None
        };
        if let Some(id) = id {
            if cpal::available_hosts().contains(&id) {
                return cpal::host_from_id(id).ok();
            }
        }
    }
    if cpal::available_hosts().contains(&HostId::Asio) {
        if let Ok(host) = cpal::host_from_id(HostId::Asio) {
            return Some(host);
        }
    }
    if cpal::available_hosts().contains(&HostId::Wasapi) {
        if let Ok(host) = cpal::host_from_id(HostId::Wasapi) {
            return Some(host);
        }
    }
    None
}

pub(super) fn select_device(
    host: &cpal::Host,
    preferred_id: Option<&str>,
    preferred_name: Option<&str>,
) -> Option<Device> {
    if let Some(id) = preferred_id {
        let parsed = id.parse::<cpal::DeviceId>().ok()?;
        return host.device_by_id(&parsed);
    }
    let mut outputs = match host.output_devices() {
        Ok(devices) => devices,
        Err(_) => return None,
    };

    if let Some(name) = preferred_name {
        if let Some(dev) = outputs.find(|d| d.to_string() == name) {
            return Some(dev);
        }
        let mut outputs2 = match host.output_devices() {
            Ok(devices) => devices,
            Err(_) => return None,
        };
        if let Some(dev) = outputs2.find(|d| d.to_string().contains(name)) {
            return Some(dev);
        }
    }

    let devices: Vec<Device> = match host.output_devices() {
        Ok(devices) => devices.collect(),
        Err(_) => return None,
    };
    let is_asio = host.id() == HostId::Asio;

    let find_by_keywords = |keywords: &[&str]| -> Option<Device> {
        devices
            .iter()
            .find(|dev| {
                let lower = dev.to_string().to_lowercase();
                keywords.iter().any(|keyword| lower.contains(keyword))
            })
            .cloned()
    };

    if is_asio {
        if let Some(dev) =
            find_by_keywords(&["voicemeeter", "vb-audio", "virtual asio", "virtual cable"])
        {
            return Some(dev);
        }
        return devices.into_iter().next();
    }

    if let Some(dev) = find_by_keywords(&[
        "voicemeeter",
        "vb-audio",
        "virtual cable",
        "hifi cable",
        "hifi-cable",
    ]) {
        return Some(dev);
    }
    for dev in devices.iter() {
        let lower_name = dev.to_string().to_lowercase();
        if (lower_name.contains("speakers") || lower_name.contains("haut-parleurs"))
            && !lower_name.contains("voicemeeter")
            && !lower_name.contains("cable")
            && !lower_name.contains("steam")
            && !lower_name.contains("microphone")
        {
            return Some(dev.clone());
        }
    }

    host.default_output_device()
}
