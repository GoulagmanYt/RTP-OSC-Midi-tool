fn main() {
    // The icon participates in generated Windows resource metadata.
    println!("cargo:rerun-if-changed=icons");
    // Wry uses the version-6 common-controls subclass APIs. Rust's unit test
    // harness also links these APIs when diagnostics own UI handles.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        // Tauri already supplies a manifest resource for application binaries.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    tauri_build::build()
}
