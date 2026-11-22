fn main() {
    #[cfg(feature = "mpv")]
    {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    tauri_build::build()
}
