//! `RUSTC_WRAPPER` half of `callscope-index`.
//!
//! The orchestrator ([`crate::orchestrate`]) drives the target workspace's
//! `cargo test --no-run` build with this same binary set as `RUSTC_WRAPPER`.
//! Cargo then invokes us once per crate as `callscope-index <rustc> <args...>`.
//!
//! For a workspace-member ("primary") crate we run an in-process `rustc_public`
//! driver that both (a) compiles the crate normally — so cargo's build
//! completes and dependents link — and (b) extracts a call-graph
//! [`Fragment`](crate::graph_build::Fragment) in the `after_analysis` callback,
//! writing it into the shared output directory. For every other invocation
//! (dependencies, build scripts, proc-macros, version probes) we transparently
//! exec the real `rustc`, so we never pay the driver cost or risk mis-handling
//! a crate we do not index.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use crate::graph_build;

/// Entry point when invoked as the compiler wrapper.
pub fn wrapper_main() -> ExitCode {
    // argv: [self, <real-rustc>, <rustc args...>]
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        eprintln!("callscope-index (wrapper): missing rustc path");
        return ExitCode::FAILURE;
    }
    let real_rustc = argv[1].clone();
    let rustc_args = &argv[2..];

    if should_extract(rustc_args) {
        run_driver_and_extract(&real_rustc, rustc_args)
    } else {
        passthrough(&real_rustc, rustc_args)
    }
}

/// Only index workspace-member compilations: a primary package, an actual
/// crate compile (not a `--print`/`-vV` probe), not a build script, not a
/// proc-macro.
fn should_extract(args: &[String]) -> bool {
    let primary = std::env::var("CARGO_PRIMARY_PACKAGE").as_deref() == Ok("1");
    if !primary {
        return false;
    }
    let crate_name = arg_value(args, "--crate-name");
    let Some(name) = crate_name else { return false };
    if name.starts_with("build_script") {
        return false;
    }
    if is_proc_macro(args) {
        return false;
    }
    // A real compile names an input file; probes (`-vV`, `--print`) do not.
    args.iter().any(|a| a.ends_with(".rs"))
}

fn is_proc_macro(args: &[String]) -> bool {
    args.iter().enumerate().any(|(i, a)| {
        a == "--crate-type" && args.get(i + 1).map(String::as_str) == Some("proc-macro")
            || a == "--crate-type=proc-macro"
    })
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    let prefix = format!("{flag}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == flag {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&prefix).map(str::to_string)
        }
    })
}

/// Transparently run the real compiler.
fn passthrough(real_rustc: &str, args: &[String]) -> ExitCode {
    let status = Command::new(real_rustc).args(args).status();
    match status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("callscope-index: failed to exec rustc: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Run the in-process `rustc_public` driver, extracting a fragment after
/// analysis, then let compilation finish so cargo gets its outputs.
fn run_driver_and_extract(real_rustc: &str, rustc_args: &[String]) -> ExitCode {
    // The driver is our own binary, not the toolchain's rustc, so it cannot
    // infer the sysroot from its path. Supply it explicitly.
    let mut driver_args: Vec<String> = Vec::with_capacity(rustc_args.len() + 3);
    driver_args.push(real_rustc.to_string());
    driver_args.extend(rustc_args.iter().cloned());
    if arg_value(rustc_args, "--sysroot").is_none() {
        if let Some(sysroot) = sysroot(real_rustc) {
            driver_args.push("--sysroot".to_string());
            driver_args.push(sysroot);
        }
    }

    let crate_name = arg_value(rustc_args, "--crate-name").unwrap_or_else(|| "crate".to_string());

    let result = rustc_public::run_with_tcx!(&driver_args, |tcx| {
        let fragment = graph_build::extract(tcx);
        write_fragment(&crate_name, &fragment);
        std::ops::ControlFlow::Continue::<(), ()>(())
    });

    match result {
        Ok(()) | Err(rustc_public::CompilerError::Skipped) => ExitCode::SUCCESS,
        Err(rustc_public::CompilerError::Interrupted(())) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Ask the real rustc for its sysroot.
fn sysroot(real_rustc: &str) -> Option<String> {
    let out = Command::new(real_rustc).arg("--print").arg("sysroot").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Write this crate's fragment into the shared output directory. The filename
/// is unique per invocation (crate + pid + monotonic tag) so parallel cargo
/// jobs never clobber each other.
fn write_fragment(crate_name: &str, fragment: &graph_build::Fragment) {
    let Ok(dir) = std::env::var("CALLSCOPE_OUT_DIR") else {
        eprintln!("callscope-index (wrapper): CALLSCOPE_OUT_DIR unset; dropping fragment");
        return;
    };
    let dir = PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("callscope-index (wrapper): cannot create out dir: {e}");
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let file = dir.join(format!("{crate_name}-{}-{nanos}.json", std::process::id()));
    match serde_json::to_vec(fragment) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&file, bytes) {
                eprintln!("callscope-index (wrapper): cannot write fragment: {e}");
            }
        }
        Err(e) => eprintln!("callscope-index (wrapper): cannot serialize fragment: {e}"),
    }
}
