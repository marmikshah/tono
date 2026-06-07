//! Sonarium binary entry point.
//!
//! Grows into the MCP server bootstrap (stdio / HTTP transports); for now it
//! only identifies itself so the scaffold is runnable end to end.

fn main() {
    println!("sonarium {}", env!("CARGO_PKG_VERSION"));
}
