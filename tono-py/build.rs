// A pyo3 extension module leaves the CPython symbols undefined at link time (the
// interpreter provides them on import). On macOS that needs an explicit linker
// flag; `rustc-cdylib-link-arg` scopes it to *this* crate's cdylib, so a plain
// `cargo build` / `cargo build --workspace` links tono-py without touching the
// rest of the workspace. (maturin sets this itself, but we support cargo too.)
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-undefined");
        println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
}
