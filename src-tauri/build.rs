fn main() {
    #[cfg(feature = "mpv")]
    {
        // 链接 src-tauri/lib/mpv.lib，并把 libmpv-2.dll 复制到 exe 旁边
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rerun-if-changed=lib/libmpv-2.dll");
        if let Ok(out) = std::env::var("OUT_DIR") {
            // OUT_DIR = target/<profile>/build/<pkg>/out
            if let Some(profile_dir) = std::path::Path::new(&out).ancestors().nth(3) {
                let _ = std::fs::copy(dir.join("libmpv-2.dll"), profile_dir.join("libmpv-2.dll"));
            }
        }
    }
    tauri_build::build()
}
