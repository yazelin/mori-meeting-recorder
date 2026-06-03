//! 列舉收音裝置:輸入(麥)+ 系統源(monitor)。Linux 走 pactl、Windows 走 cpal。
//! 解析與平台呼叫分離 → 純解析函式可單測。

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceInfo {
    pub id: String,    // 開裝置用的技術名(pulse source name / cpal device name)
    pub label: String, // 友善顯示名
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AudioDevices {
    pub inputs: Vec<DeviceInfo>,
    pub system_sources: Vec<DeviceInfo>,
}

/// 解析 `pactl list sources`(verbose)→ name→Description map。
#[cfg(any(target_os = "linux", test))]
fn parse_descriptions(verbose: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut cur_name: Option<String> = None;
    for line in verbose.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Name: ") {
            cur_name = Some(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("Description: ") {
            if let Some(n) = &cur_name {
                map.insert(n.clone(), rest.trim().to_string());
            }
        }
    }
    map
}

/// 純函式:把 `pactl list short sources` + verbose 解析成 AudioDevices。
/// short 每行第 2 欄=name;`.monitor` 結尾→system_sources、否則→inputs。label 取 Description,缺則用 name。
#[cfg(any(target_os = "linux", test))]
pub fn parse_pactl(short: &str, verbose: &str) -> AudioDevices {
    let desc = parse_descriptions(verbose);
    let mut inputs = Vec::new();
    let mut system_sources = Vec::new();
    for line in short.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[1];
        let label = desc.get(name).cloned().unwrap_or_else(|| name.to_string());
        let info = DeviceInfo { id: name.to_string(), label };
        if name.ends_with(".monitor") {
            system_sources.push(info);
        } else {
            inputs.push(info);
        }
    }
    AudioDevices { inputs, system_sources }
}

#[cfg(target_os = "linux")]
pub fn list_devices() -> AudioDevices {
    use std::process::Command;
    let short = Command::new("pactl").args(["list", "short", "sources"]).output();
    let verbose = Command::new("pactl").args(["list", "sources"]).output();
    let short = short.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let verbose = verbose.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    parse_pactl(&short, &verbose)
}

#[cfg(target_os = "windows")]
pub fn list_devices() -> AudioDevices {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let to_info = |d: &cpal::Device| {
        let name = d.name().unwrap_or_else(|_| "(unknown)".into());
        DeviceInfo { id: name.clone(), label: name }
    };
    let inputs = host.input_devices().map(|it| it.map(|d| to_info(&d)).collect()).unwrap_or_default();
    let system_sources = host.output_devices().map(|it| it.map(|d| to_info(&d)).collect()).unwrap_or_default();
    AudioDevices { inputs, system_sources }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn list_devices() -> AudioDevices {
    AudioDevices { inputs: Vec::new(), system_sources: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pactl_splits_inputs_and_monitors_with_descriptions() {
        let short = "\
1062\talsa_input.pci-0000.analog-stereo\tPipeWire\ts16le\tSUSPENDED
1064\talsa_input.usb-fifine.mono-fallback\tPipeWire\ts24le\tSUSPENDED
1061\talsa_output.pci-0000.hdmi-stereo.monitor\tPipeWire\ts16le\tSUSPENDED";
        let verbose = "\
Source #1064
\tName: alsa_input.usb-fifine.mono-fallback
\tDescription: fifine Microphone Mono
Source #1061
\tName: alsa_output.pci-0000.hdmi-stereo.monitor
\tDescription: Monitor of HDMI";
        let d = parse_pactl(short, verbose);
        assert_eq!(d.inputs.len(), 2);
        assert_eq!(d.system_sources.len(), 1);
        // 友善名稱套用
        let fifine = d.inputs.iter().find(|x| x.id.contains("fifine")).unwrap();
        assert_eq!(fifine.label, "fifine Microphone Mono");
        // 無 Description 的退技術名
        let builtin = d.inputs.iter().find(|x| x.id.contains("analog-stereo")).unwrap();
        assert_eq!(builtin.label, builtin.id);
        // monitor 進 system_sources
        assert!(d.system_sources[0].id.ends_with(".monitor"));
    }

    #[test]
    fn parse_pactl_empty_input_yields_empty() {
        let d = parse_pactl("", "");
        assert!(d.inputs.is_empty() && d.system_sources.is_empty());
    }
}
