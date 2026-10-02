// SPDX-License-Identifier: GPL-2.0-or-later
//! Compiles the Qt dock, settings dialog and chime, and links against OBS
//! (libobs, obs-frontend-api, Qt).
//!
//! macOS: OBS's own frameworks (see `macos`). Windows: Qt from obs-deps and
//! import libraries generated from the OBS release, both prepared by
//! scripts/bootstrap-windows.ps1.
//!
//! Env overrides: OBS_APP, OBS_APP_ARM64 / OBS_APP_X86_64 (macOS),
//! OBS_LIB_DIR (Windows), QT6_DEPS_DIR.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let qt = env_path("QT6_DEPS_DIR").unwrap_or_else(|| root.join("third_party/obs-deps-qt6"));
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        windows(&root, &qt);
    } else {
        macos(&root, &qt);
    }
    println!("cargo:rerun-if-changed=dock");
    println!("cargo:rerun-if-env-changed=OBS_APP");
    println!("cargo:rerun-if-env-changed=QT6_DEPS_DIR");
}

fn macos(root: &Path, qt: &Path) {
    let qt_lib = qt.join("lib");
    if !qt_lib.join("QtWidgets.framework/Headers").is_dir() {
        panic!("Qt headers not found in {}. Run scripts/bootstrap-macos.sh first.", qt_lib.display());
    }
    // OBS's libobs is single-architecture, so each slice of a universal
    // plugin links against the matching OBS build: OBS_APP_ARM64 /
    // OBS_APP_X86_64, else OBS_APP, else third_party/obs-app-<arch> (from
    // scripts/fetch-obs-macos.sh), else the installed OBS.
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64".to_string(),
        other => other.to_string(),
    };
    let per_arch_var = format!("OBS_APP_{}", arch.to_uppercase());
    println!("cargo:rerun-if-env-changed={per_arch_var}");
    let fetched = root.join(format!("third_party/obs-app-{arch}"));
    let obs = env_path(&per_arch_var)
        .or_else(|| env_path("OBS_APP"))
        .or_else(|| fetched.is_dir().then_some(fetched))
        .unwrap_or_else(|| PathBuf::from("/Applications/OBS.app"));
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
}

fn windows(root: &Path, qt: &Path) {
    let qt_include = qt.join("include");
    if !qt.join("lib/Qt6Widgets.lib").is_file() {
        panic!("Qt not found in {}. Run scripts\\bootstrap-windows.ps1 first.", qt.display());
    }
    let obs_lib = env_path("OBS_LIB_DIR").unwrap_or_else(|| root.join("third_party/obs-x64/lib"));
    println!("cargo:rerun-if-env-changed=OBS_LIB_DIR");

    let mut dock = cc::Build::new();
    dock.cpp(true)
        .file("dock/sp_dock.cpp")
        .file("dock/sp_settings.cpp")
        .std("c++17")
        .include(&qt_include)
        // Qt 6 requires these with MSVC; the sources contain UTF-8 symbols.
        .flag("/Zc:__cplusplus")
        .flag("/permissive-")
        .flag("/utf-8")
        .flag("/EHsc")
        .define("UNICODE", None)
        .define("_UNICODE", None)
        .warnings(true);
    for module in ["QtCore", "QtGui", "QtWidgets"] {
        dock.include(qt_include.join(module));
    }
    dock.compile("sp_dock");

    cc::Build::new().file("dock/sp_chime.c").flag("/utf-8").warnings(true).compile("sp_chime");

    println!("cargo:rustc-link-search=native={}", qt.join("lib").display());
    for lib in ["Qt6Widgets", "Qt6Gui", "Qt6Core"] {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rustc-link-search=native={}", obs_lib.display());
    for lib in ["obs", "obs-frontend-api", "winmm"] {
        println!("cargo:rustc-link-lib={lib}");
    }
}

fn env_path(var: &str) -> Option<PathBuf> {
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}
