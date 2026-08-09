fn main() {
    // The icon participates in generated Windows resource metadata.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
