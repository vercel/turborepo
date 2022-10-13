fn main() {
    println!("cargo:rustc-link-search=../cli");
    println!("cargo:rustc-link-lib=turbo");
    println!("cargo:rustc-link-lib=framework=cocoa");
    println!("cargo:rustc-link-lib=framework=security");
}
