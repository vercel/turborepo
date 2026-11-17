//! Native link inputs declared by a build script.
fn main() {
    println!("cargo:rustc-link-search=native=native/lib");
    println!("cargo:rustc-link-lib=static=hello");
    println!("cargo:rustc-link-lib=dylib=ssl");
    println!("cargo:rerun-if-changed=native/hello.c");
}
