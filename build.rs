//! Embeds the Quadrant icon and the version resource in the Windows exe.
//!
//! gpui links its own resource file with the application manifest, so this
//! one declares no manifest of its own. gpui's window class loads icon
//! resource 1 for the title bar and the taskbar.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/icons/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest_dir.join("packaging/icons/icon.ico");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = [
        "CARGO_PKG_VERSION_MAJOR",
        "CARGO_PKG_VERSION_MINOR",
        "CARGO_PKG_VERSION_PATCH",
    ]
    .map(|key| env::var(key).unwrap())
    .join(",");
    let rc = format!(
        r#"#include <winver.h>
1 ICON "{icon}"
VS_VERSION_INFO VERSIONINFO
FILEVERSION {numeric},0
PRODUCTVERSION {numeric},0
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "MrQuantumOFF (Demir Yerli)"
      VALUE "FileDescription", "Quadrant-Native"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "quadrant-native"
      VALUE "LegalCopyright", "Copyright (C) 2026 MrQuantumOFF (Demir Yerli)"
      VALUE "OriginalFilename", "quadrant-native.exe"
      VALUE "ProductName", "Quadrant-Native"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 0x04B0
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("quadrant-native.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
