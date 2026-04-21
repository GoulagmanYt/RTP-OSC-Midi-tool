pub use crate::plugin_probe::{
    default_vst_scan_roots, scan_vst_plugins_in_roots,
};

pub fn is_vst2_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("dll") || ext.eq_ignore_ascii_case("vst"))
        .unwrap_or(false)
}

pub fn is_vst3_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("vst3"))
        .unwrap_or(false)
}
