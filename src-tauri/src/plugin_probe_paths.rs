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
    let mut visited_dirs = HashSet::new();
    let mut visited_plugins = HashSet::new();
    scan_dir(root, &mut out, &mut visited_dirs, &mut visited_plugins, 0);
    out
}

#[derive(Hash, PartialEq, Eq)]
pub(super) enum PluginIdentity {
    #[cfg(windows)]
    File {
        volume: u32,
        index: u64,
    },
    Path(PathBuf),
}

pub(super) fn plugin_identity(path: &Path) -> PluginIdentity {
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
            },
        };
        // Open the target, including VST3 bundle directories. Volume + file
        // index also identify hard links, which canonical path spelling cannot.
        if let Ok(file) = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
        {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            if unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
                .is_ok()
            {
                return PluginIdentity::File {
                    volume: info.dwVolumeSerialNumber,
                    index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
                };
            }
        }
    }
    PluginIdentity::Path(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn scan_dir(
    root: &Path,
    out: &mut Vec<PathBuf>,
    visited_dirs: &mut HashSet<PathBuf>,
    visited_plugins: &mut HashSet<PluginIdentity>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    if let Ok(canonical) = fs::canonicalize(root) {
        if !visited_dirs.insert(canonical) {
            return;
        }
    } else if !visited_dirs.insert(root.to_path_buf()) {
        return;
    }

    let Ok(read_dir) = fs::read_dir(root) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // A selected root may itself be a junction, but never descend into
            // directory reparse points found underneath it. This also bounds
            // traversal when canonicalization cannot resolve a junction cycle.
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
                if fs::symlink_metadata(&path).map_or(true, |metadata| {
                    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
                }) {
                    continue;
                }
            }
            if is_vst3_path(&path) {
                if visited_plugins.insert(plugin_identity(&path)) {
                    out.push(path);
                }
            } else {
                scan_dir(&path, out, visited_dirs, visited_plugins, depth + 1);
            }
            continue;
        }

        if (is_vst2_path(&path) || is_vst3_path(&path))
            && visited_plugins.insert(plugin_identity(&path))
        {
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

/// Returns the path spelling passed to third-party plugin loaders.
///
/// OSCMidi keeps canonical paths for identity and caching, but some legacy
/// Windows plugins parse the Win32 verbatim prefix (`\\?\`) themselves and
/// crash before returning an instance.
pub fn native_plugin_load_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
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

#[cfg(test)]
mod tests {
    use super::native_plugin_load_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn strips_windows_verbatim_drive_prefix_for_native_loaders() {
        assert_eq!(
            native_plugin_load_path(Path::new(r"\\?\C:\VST\Church Organ.dll")),
            PathBuf::from(r"C:\VST\Church Organ.dll")
        );
    }

    #[test]
    fn converts_windows_verbatim_unc_prefix_for_native_loaders() {
        assert_eq!(
            native_plugin_load_path(Path::new(r"\\?\UNC\server\share\Piano.vst3")),
            PathBuf::from(r"\\server\share\Piano.vst3")
        );
    }

    #[test]
    fn preserves_normal_native_plugin_path() {
        let path = Path::new(r"C:\VST\Piano.vst3");
        assert_eq!(native_plugin_load_path(path), path);
    }
}
