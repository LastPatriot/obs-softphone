// SPDX-License-Identifier: GPL-2.0-or-later
//! Compiles the Qt dock and links against the OBS installation the plugin will
//! be loaded into (macOS dev builds; packaging is M5).
//!
//! Env overrides: OBS_APP (default /Applications/OBS.app), QT6_DEPS_DIR.

use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let qt = env_path("QT6_DEPS_DIR").unwrap_or_else(|| root.join("third_party/obs-deps-qt6"));
    let qt_lib = qt.join("lib");
    if !qt_lib.join("QtWidgets.framework/Headers").is_dir() {
        panic!("Qt headers not found in {}. Run scripts/bootstrap-macos.sh first.", qt_lib.display());
    }
    let obs = env_path("OBS_APP").unwrap_or_else(|| PathBuf::from("/Applications/OBS.app"));
    let fw = obs.join("Contents/Frameworks");

    cc::Build::new()
        .cpp(true)
        .file("dock/sp_dock.cpp")
        .file("dock/sp_settings.cpp")
        .std("c++17")
        .flag(format!("-F{}", qt_lib.display()))
        .flag("-mmacosx-version-min=12.0")
        .warnings(true)
        .compile("sp_dock");

    cc::Build::new()
        .file("dock/sp_chime.c")
        .flag("-mmacosx-version-min=12.0")
        .warnings(true)
        .compile("sp_chime");
    for f in ["AudioToolbox", "CoreFoundation"] {
        println!("cargo:rustc-link-lib=framework={f}");
    }

    println!("cargo:rustc-link-search=framework={}", fw.display());
    for f in ["libobs", "QtWidgets", "QtGui", "QtCore"] {
        println!("cargo:rustc-link-lib=framework={f}");
    }
    println!("cargo:rustc-link-arg={}", fw.join("obs-frontend-api.dylib").display());
    println!("cargo:rustc-link-lib=c++");
    // OBS's own rpath resolves these; this one helps tools like otool/dyld_info.
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-install_name,@rpath/obs-softphone.plugin/Contents/MacOS/obs-softphone");

    println!("cargo:rerun-if-changed=dock");
    println!("cargo:rerun-if-env-changed=OBS_APP");
    println!("cargo:rerun-if-env-changed=QT6_DEPS_DIR");
}

fn env_path(var: &str) -> Option<PathBuf> {
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}
