fn main() {
    println!("cargo:rustc-link-search=../cli");
    println!("cargo:rustc-link-lib=turbo");
    let bindings = bindgen::Builder::default()
        .header("../cli/libturbo.h")
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .allowlist_function("nativeRunWithArgs")
        .allowlist_function("testBindgen")
        .allowlist_type("GoString")
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file("src/ffi.rs")
        .expect("Couldn't write bindings!");
    println!("cargo:rustc-link-lib=framework=cocoa");
    println!("cargo:rustc-link-lib=framework=security");
}
