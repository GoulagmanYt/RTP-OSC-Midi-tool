fn main() {
    // Make sure Cargo reruns the build script when the app icon changes
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
