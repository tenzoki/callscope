// P7: stdio MCP server exposing C1–C8 as tools. Loads the on-disk index once
// and runs the staleness check on every request. Links no compiler internals,
// so it builds on stable Rust. The MCP SDK (rmcp) is wired in P7.
//
// P1 scaffold: a stub that prints and exits 0. No server logic yet.

fn main() {
    eprintln!("callscope-mcp: not yet implemented (P1 scaffold)");
    std::process::exit(0);
}
