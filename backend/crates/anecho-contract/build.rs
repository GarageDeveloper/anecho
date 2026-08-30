//! Generates Rust types from `contract/*.proto` at build time.
//!
//! The `contract/` directory is the single source of truth (see CLAUDE.md rule 3).
//! Generated code lands in `OUT_DIR` and is included by `src/lib.rs`; nothing generated is
//! ever committed.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let contract_dir = manifest_dir.join("../../../contract");
    let contract_dir = contract_dir
        .canonicalize()
        .expect("contract/ directory must exist at the repository root");

    let mut protos: Vec<PathBuf> = std::fs::read_dir(&contract_dir)
        .expect("read contract/")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "proto"))
        .collect();
    protos.sort();
    assert!(
        !protos.is_empty(),
        "no .proto file found in {}",
        contract_dir.display()
    );

    for p in &protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed={}", contract_dir.display());

    // Use the vendored protoc so contributors and CI need no system install.
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");

    prost_build::Config::new()
        .protoc_executable(protoc)
        .compile_protos(&protos, &[contract_dir])
        .expect("protobuf compilation failed");
}
