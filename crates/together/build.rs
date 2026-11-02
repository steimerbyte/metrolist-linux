//! Generates the Listen Together wire types from the upstream proto.
//!
//! `protox` is a pure-Rust compiler, so no `protoc` binary is required.

fn main() {
    println!("cargo:rerun-if-changed=proto/listentogether.proto");
    let descriptor_set = protox::compile(["proto/listentogether.proto"], ["proto"])
        .expect("compile listentogether.proto");
    prost_build::Config::new()
        .compile_fds(descriptor_set)
        .expect("generate listentogether.rs");
}
