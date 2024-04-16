fn main() {
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    napi_build::setup();

    // This is a workaround for napi always including a GCC specific flag.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        println!("cargo:rerun-if-env-changed=DEBUG_GENERATED_CODE");
        println!("cargo:rerun-if-env-changed=TYPE_DEF_TMP_PATH");
        println!("cargo:rerun-if-env-changed=CARGO_CFG_NAPI_RS_CLI_VERSION");

        println!("cargo:rustc-cdylib-link-arg=-undefined");
        println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
}
