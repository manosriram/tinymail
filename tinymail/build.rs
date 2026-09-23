fn main() {
    // tauri-build never watches frontendDist itself — without this, editing
    // ui/*.js or ui/*.html doesn't trigger a rebuild, so `cargo run` keeps
    // serving whatever was embedded last time a Rust source file changed.
    println!("cargo:rerun-if-changed=../ui");
    tauri_build::build()
}
