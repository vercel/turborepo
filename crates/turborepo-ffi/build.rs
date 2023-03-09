use std::{env, io::Result};

use cbindgen::Language;

fn main() -> Result<()> {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_language(Language::C)
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file("bindings.h");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target = env::var("TARGET").unwrap();

    match env::var("CARGO_CFG_TARGET_OS").unwrap().as_ref() {
        "linux" => {
            // statically link libunwind if compiling for musl, dynamically link otherwise
            if env::var("CARGO_FEATURE_UNWIND").is_ok() {
                println!("cargo:rustc-cfg=use_libunwind");
                if env::var("CARGO_CFG_TARGET_ENV").unwrap() == "musl"
                    && env::var("CARGO_CFG_TARGET_VENDOR").unwrap() != "alpine"
                {
                    println!("cargo:rustc-link-search=native=/usr/local/lib");
                    println!(
                        "cargo:rustc-link-search=native=/usr/local/musl/{}/lib",
                        target
                    );
                    println!("cargo:rustc-link-lib=static=z");
                    println!("cargo:rustc-link-lib=static=unwind");
                    println!("cargo:rustc-link-lib=static=unwind-ptrace");
                    println!("cargo:rustc-link-lib=dylib=unwind-{}", target_arch);
                }
            }
        }
        _ => {}
    }

    prost_build::compile_protos(&["messages.proto"], &["."])?;
    Ok(())
}
