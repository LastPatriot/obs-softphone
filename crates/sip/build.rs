// SPDX-License-Identifier: GPL-2.0-or-later
//! Compiles the C shim and links the pjproject built by scripts/bootstrap-macos.sh
//! for the target architecture (third_party/pjproject-<arch>, opus-<arch>).
//!
//! Env overrides: PJPROJECT_DIR (install prefix), OPUS_DIR.

use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    // The bootstrap script names directories after Apple's arch names.
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64".to_string(),
        other => other.to_string(),
    };
    let tp = root.join("third_party");
    let prefix = env_path("PJPROJECT_DIR").unwrap_or_else(|| tp.join(format!("pjproject-{arch}")));
    let pc_path = prefix.join("lib/pkgconfig/libpjproject.pc");
    let pc = std::fs::read_to_string(&pc_path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nRun ARCHS={arch} scripts/bootstrap-macos.sh first (or set PJPROJECT_DIR).",
            pc_path.display()
        )
    });

    let field = |name: &str| -> Vec<String> {
        pc.lines()
            .find_map(|l| l.strip_prefix(&format!("{name}:")))
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    };

    let mut build = cc::Build::new();
    build.file("shim/sp_shim.c").include("shim").warnings(true);
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
                // Static so the plugin doesn't depend on Homebrew at runtime.
                _ => println!("cargo:rustc-link-lib=static={lib}"),
            }
        }
    }

    let opus = env_path("OPUS_DIR").unwrap_or_else(|| tp.join(format!("opus-{arch}")));
    println!("cargo:rustc-link-search=native={}", opus.join("lib").display());
    println!("cargo:rerun-if-env-changed=OPUS_DIR");

    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-changed={}", pc_path.display());
    println!("cargo:rerun-if-env-changed=PJPROJECT_DIR");
}

fn env_path(var: &str) -> Option<PathBuf> {
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}
