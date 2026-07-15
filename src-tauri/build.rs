fn main() {
    println!("cargo:rerun-if-env-changed=GAMETWEAKS_BUILD_VERSION");
    tauri_build::build();
}
