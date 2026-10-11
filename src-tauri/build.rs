fn main() {
    // Listing the app's commands generates `allow-<command>` permissions, so each window's
    // capability file grants only what that window needs (SPEC §14.3).
    let manifest = tauri_build::AppManifest::new().commands(&["get_config", "update_settings"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
