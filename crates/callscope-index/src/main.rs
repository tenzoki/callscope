// P4: rustc-driver indexing engine. This is the ONLY crate that links the
// compiler (rustc_public / rustc-dev), which is why the workspace pins a
// nightly toolchain in rust-toolchain.toml. It will run each workspace crate
// through mono-item collection, build the resolved call graph, and write the
// on-disk index plus a fingerprint manifest.
//
// P1 scaffold: a stub that prints and exits 0. No driver logic yet.

fn main() {
    eprintln!("callscope-index: not yet implemented (P1 scaffold)");
    std::process::exit(0);
}
