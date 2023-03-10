use std::io::Result;

use cbindgen::Language;

fn main() -> Result<()> {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_language(Language::C)
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file("bindings.h");

    prost_build::compile_protos(&["messages.proto"], &["."])?;
    Ok(())
}
