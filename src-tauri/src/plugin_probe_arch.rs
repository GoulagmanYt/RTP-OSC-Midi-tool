use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::plugin_probe_paths::{is_vst2_path, is_vst3_path};

pub fn detect_plugin_architecture(path: &Path) -> String {
    if is_vst2_path(path) {
        return read_pe_machine(path).unwrap_or("unknown".to_string());
    }
    if is_vst3_path(path) {
        return detect_vst3_architecture(path);
    }
    "unknown".to_string()
}

pub fn detect_vst3_architecture(path: &Path) -> String {
    if path.is_file() {
        return read_pe_machine(path).unwrap_or("unknown".to_string());
    }

    let x64_dir = path.join("Contents").join("x86_64-win");
    if x64_dir.exists() {
        return "x64".to_string();
    }
    let x86_dir = path.join("Contents").join("x86-win");
    if x86_dir.exists() {
        return "x86".to_string();
    }

    if let Some(binary) = find_first_vst3_binary(path) {
        return read_pe_machine(&binary).unwrap_or("unknown".to_string());
    }

    "unknown".to_string()
}

fn find_first_vst3_binary(path: &Path) -> Option<PathBuf> {
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }
            if is_vst3_path(&entry_path) {
                return Some(entry_path);
            }
        }
    }
    None
}

pub fn read_pe_machine(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut mz = [0u8; 2];
    file.read_exact(&mut mz).ok()?;
    if &mz != b"MZ" {
        return None;
    }

    file.seek(SeekFrom::Start(0x3c)).ok()?;
    let mut pe_offset = [0u8; 4];
    file.read_exact(&mut pe_offset).ok()?;
    let pe_offset = u32::from_le_bytes(pe_offset) as u64;

    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut signature = [0u8; 4];
    file.read_exact(&mut signature).ok()?;
    if &signature != b"PE\0\0" {
        return None;
    }

    let mut machine = [0u8; 2];
    file.read_exact(&mut machine).ok()?;
    Some(match u16::from_le_bytes(machine) {
        0x14c => "x86".to_string(),
        0x8664 => "x64".to_string(),
        value => format!("0x{value:04X}"),
    })
}
