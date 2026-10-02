// SPDX-License-Identifier: GPL-2.0-or-later
//! Compiles the C shim and links pjproject and Opus, as built by
//! scripts/bootstrap-macos.sh (third_party/pjproject-<arch>, opus-<arch>) or
//! scripts/bootstrap-windows.ps1 (third_party/pjproject-x64, opus-x64).
//!
//! Env overrides: PJPROJECT_DIR (install prefix, or the source tree on
//! Windows), OPUS_DIR.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let tp = root.join("third_party");
    let mut build = cc::Build::new();
    build.file("shim/sp_shim.c").include("shim").warnings(true);

    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        windows(&tp, &mut build);
    } else {
        unix(&tp, &mut build);
    }

    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-env-changed=PJPROJECT_DIR");
    println!("cargo:rerun-if-env-changed=OPUS_DIR");
}

/// macOS (and later Linux): pjproject's autotools install and its .pc file.
fn unix(tp: &Path, build: &mut cc::Build) {
    // The bootstrap script names directories after Apple's arch names.
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64".to_string(),
        other => other.to_string(),
    };
    let prefix = env_path("PJPROJECT_DIR").unwrap_or_else(|| tp.join(format!("pjproject-{arch}")));
    let pc_path = prefix.join("lib/pkgconfig/libpjproject.pc");
    let pc = std::fs::read_to_string(&pc_path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nRun ARCHS={arch} scripts/bootstrap-macos.sh first (or set PJPROJECT_DIR).",
            pc_path.display()
        )
    });
    println!("cargo:rerun-if-changed={}", pc_path.display());

    let field = |name: &str| -> Vec<String> {
        pc.lines()
            .find_map(|l| l.strip_prefix(&format!("{name}:")))
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    };

    for flag in field("Cflags") {
        if let Some(dir) = flag.strip_prefix("-I") {
            build.include(dir);
        } else if let Some(def) = flag.strip_prefix("-D") {
            let (k, v) = def.split_once('=').map_or((def, None), |(k, v)| (k, Some(v)));
            build.define(k, v);
        }
    }
    build.compile("sp_shim");

    let mut libs = field("Libs");
    libs.extend(field("Libs.private"));
    let mut tokens = libs.into_iter();
    while let Some(tok) = tokens.next() {
        if let Some(dir) = tok.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if tok == "-framework" {
            if let Some(fw) = tokens.next() {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        } else if let Some(lib) = tok.strip_prefix("-l") {
            match lib {
                "m" | "pthread" => println!("cargo:rustc-link-lib={lib}"),
                // Static, so the plugin has no runtime dependencies.
                _ => println!("cargo:rustc-link-lib=static={lib}"),
            }
        }
    }

    let opus = env_path("OPUS_DIR").unwrap_or_else(|| tp.join(format!("opus-{arch}")));
    println!("cargo:rustc-link-search=native={}", opus.join("lib").display());
}

/// Windows: pjproject's Visual Studio build (its source tree, "libpjproject"
/// aggregate library, /MD) with Schannel TLS.
fn windows(tp: &Path, build: &mut cc::Build) {
    let pj = env_path("PJPROJECT_DIR").unwrap_or_else(|| tp.join("pjproject-x64"));
    let lib_dir = pj.join("pjsip-apps/lib");
    let lib = std::fs::read_dir(&lib_dir)
        .ok()
        .and_then(|entries| {
            entries.filter_map(Result::ok).map(|e| e.path()).find(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with("libpjproject-") && name.ends_with("Release-Dynamic.lib")
            })
        })
        .unwrap_or_else(|| {
            panic!("libpjproject not found in {}. Run scripts\\bootstrap-windows.ps1 first.", lib_dir.display())
        });
    println!("cargo:rerun-if-changed={}", lib.display());

    for module in ["pjlib", "pjlib-util", "pjnath", "pjmedia", "pjsip"] {
        build.include(pj.join(module).join("include"));
    }
    build.compile("sp_shim");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static={}", lib.file_stem().unwrap().to_string_lossy());

    let opus = env_path("OPUS_DIR").unwrap_or_else(|| tp.join("opus-x64"));
    println!("cargo:rustc-link-search=native={}", opus.join("lib").display());
    println!("cargo:rustc-link-lib=static=opus");

    // What pjproject (sockets, audio, Schannel TLS, GUIDs) needs from Windows.
    for sys in ["ws2_32", "iphlpapi", "winmm", "ole32", "oleaut32", "uuid", "advapi32", "user32", "crypt32", "secur32", "ncrypt", "bcrypt"] {
        println!("cargo:rustc-link-lib={sys}");
    }
}

fn env_path(var: &str) -> Option<PathBuf> {
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}
