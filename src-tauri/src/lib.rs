//! Infodock core: app builder, windows and plugins (SPEC §4.2).

use tauri::{WebviewUrl, WebviewWindowBuilder};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            WebviewWindowBuilder::new(app, "bar", WebviewUrl::App("bar".into()))
                .title("Infodock")
                .decorations(false)
                .transparent(true)
                .skip_taskbar(true)
                .resizable(false)
                .shadow(false)
                .always_on_top(true)
                .inner_size(800.0, 28.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Infodock");
}
