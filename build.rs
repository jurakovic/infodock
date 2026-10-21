use std::path::{Path, PathBuf};

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint UI compiles");

    // Build scripts run on the host, so check the target rather than `cfg(windows)`.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

/// Embeds the exe icon, the application manifest and version information, into the test
/// executables too: they then load as the app does (common controls 6, which alone has
/// `TaskDialogIndirect`, and the DPI awareness), so code that reaches native dialogs can be
/// tested.
fn windows_resources() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    let version = std::env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let icon = manifest_dir.join("icons/icon.ico");
    let manifest = manifest_dir.join("resources/infodock.manifest");
    for path in [&icon, &manifest] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let numbers: Vec<&str> = version.split(['.', '-', '+']).take(3).collect();
    let numeric = |i: usize| {
        numbers
            .get(i)
            .and_then(|n| n.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let comma = format!("{},{},{},0", numeric(0), numeric(1), numeric(2));
    // Resource IDs: 1 ICON, RT_MANIFEST (24) with id 1, VS_VERSION_INFO (1).
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {comma}
PRODUCTVERSION {comma}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Infodock"
      VALUE "FileDescription", "Infodock"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "infodock"
      VALUE "LegalCopyright", "Copyright (c) 2026 jurakovic. MIT licence."
      VALUE "OriginalFilename", "infodock.exe"
      VALUE "ProductName", "Infodock"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = rc_path(&icon),
        manifest = rc_path(&manifest),
    );
    let rc_file = out_dir.join("infodock.rc");
    std::fs::write(&rc_file, rc).expect("the resource script can be written");
    embed_resource::compile_for_everything(&rc_file, embed_resource::NONE)
        .manifest_required()
        .expect("the Windows resources compile");
}

/// A path inside an `.rc` string literal.
fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
