//! Bake the toolchain sysroot into the binary at build time.
//!
//! `callscope-index` is a `rustc_public` driver: at run time it must tell cargo
//! and its in-process compiler which sysroot to use, and it links the compiler
//! dylib from that sysroot. The only place we can reliably learn the sysroot is
//! here, where the crate is compiled under its pinned nightly (rust-toolchain.toml):
//! cargo sets `RUSTC` to the exact compiler, and `rustc --print sysroot` names
//! its sysroot. We emit it as a compile-time env var the binary reads back.

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let out = Command::new(&rustc)
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("run `rustc --print sysroot`");
    let sysroot = String::from_utf8(out.stdout)
        .expect("sysroot is utf-8")
        .trim()
        .to_string();
    println!("cargo:rustc-env=CALLSCOPE_SYSROOT={sysroot}");

    // The binary links the compiler dylib (`librustc_driver`) from the sysroot,
    // but rustc does not bake an rpath to it for a bin built outside the
    // toolchain tree. Add one so the binary loads without a DYLD/LD env var.
    let host = std::env::var("HOST").unwrap_or_default();
    println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib/rustlib/{host}/lib");

    // Rebuild if the toolchain changes underneath us.
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=build.rs");
}
