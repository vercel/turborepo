fn main() {
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-cfg=feature=\"watch_recursively\"");
        println!("cargo:rustc-cfg=feature=\"watch_ancestors\"");
    }
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-cfg=feature=\"watch_ancestors\"");
    }
}
