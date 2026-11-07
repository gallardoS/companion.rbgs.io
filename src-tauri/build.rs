fn main() {
    // Windows embeds the ICO at link time; changing it must rebuild the executable.
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-env-changed=RBGS_UPDATER_PUBLIC_KEY");
    tauri_build::build()
}
