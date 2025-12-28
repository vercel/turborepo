fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tonic_build_result = tonic_build::configure()
        .build_server(true)
        .file_descriptor_set_path("src/daemon/file_descriptor_set.bin")
        .compile(
            &["./src/daemon/proto/turbod.proto"],
            &["./src/daemon/proto"],
        );

    let invocation = std::env::var("RUSTC_WRAPPER").unwrap_or_default();
    if invocation.ends_with("rust-analyzer") {
        if tonic_build_result.is_err() {
            println!("cargo:warning=tonic_build failed, but continuing with rust-analyzer");
        }

        return Ok(());
    } else {
        tonic_build_result.expect("tonic_build command");
    }

    Ok(())
}
