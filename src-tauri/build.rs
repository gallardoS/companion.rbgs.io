fn main() {
    // Windows embeds the ICO at link time; changing it must rebuild the executable.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
