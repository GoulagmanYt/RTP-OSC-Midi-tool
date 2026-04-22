use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_VST3_PLUGIN_DIR: &str = "C:\\Program Files\\Common Files\\VST3";
pub const DEFAULT_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\VSTPlugins";
pub const STEINBERG_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\Steinberg\\VstPlugins";
pub const COMMON_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\Common Files\\VST2";
pub const COMMON_STEINBERG_VST2_PLUGIN_DIR: &str =
    "C:\\Program Files\\Common Files\\Steinberg\\VST2";
pub const DEFAULT_VST3_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Common Files\\VST3";
pub const DEFAULT_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\VSTPlugins";
pub const STEINBERG_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Steinberg\\VstPlugins";
pub const COMMON_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Common Files\\VST2";
pub const COMMON_STEINBERG_VST2_PLUGIN_DIR_X86: &str =
    "C:\\Program Files (x86)\\Common Files\\Steinberg\\VST2";

pub fn default_vst_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for raw in [
        DEFAULT_VST3_PLUGIN_DIR,
        DEFAULT_VST2_PLUGIN_DIR,
        STEINBERG_VST2_PLUGIN_DIR,
        COMMON_VST2_PLUGIN_DIR,
        COMMON_STEINBERG_VST2_PLUGIN_DIR,
        DEFAULT_VST3_PLUGIN_DIR_X86,
        DEFAULT_VST2_PLUGIN_DIR_X86,
        STEINBERG_VST2_PLUGIN_DIR_X86,
        COMMON_VST2_PLUGIN_DIR_X86,
        COMMON_STEINBERG_VST2_PLUGIN_DIR_X86,
    ] {
        let path = PathBuf::from(raw);
        if path.exists() && !roots.iter().any(|existing| existing == &path) {
            roots.push(path);
        }
    }
    roots
}

pub fn is_vst2_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("dll"))
        .unwrap_or(false)
}

pub fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("vst3"))
        .unwrap_or(false)
}

pub fn scan_plugin_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    scan_dir(root, &mut out);
    out
}

fn scan_dir(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read_dir) = fs::read_dir(root) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if is_vst3_path(&path) {
                out.push(path);
            } else {
                scan_dir(&path, out);
            }
            continue;
        }

        if is_vst2_path(&path) || is_vst3_path(&path) {
            out.push(path);
        }
    }
}

pub fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

pub fn plugin_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

pub fn unique_existing_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen_paths = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        let key = normalize_path(root);
        if seen_paths.insert(key) {
            out.push(root.clone());
        }
    }
    out
}
