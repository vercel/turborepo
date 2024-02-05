#[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
fn check_tls_config() {}
#[cfg(not(any(feature = "native-tls", feature = "rustls-tls")))]
fn check_tls_config() {
    panic!("You must enable one of the TLS features: native-tls or rustls-tls");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    check_tls_config();

    let tonic_build_result = tonic_build::configure()
        .build_server(true)
        .file_descriptor_set_path("src/daemon/file_descriptor_set.bin")
        .compile(&["turbod.proto"], &["./src/daemon"]);
    let capnpc_result = capnpc::CompilerCommand::new()
        .file("./src/hash/proto.capnp")
        .import_path("./src/hash/std") // we need to include the 'stdlib' for capnp-go
        .default_parent_module(vec!["hash".to_string()])
        .run();

    let invocation = std::env::var("RUSTC_WRAPPER").unwrap_or_default();
    if invocation.ends_with("rust-analyzer") {
        if tonic_build_result.is_err() {
            println!("cargo:warning=tonic_build failed, but continuing with rust-analyzer");
        }

        if capnpc_result.is_err() {
            println!("cargo:warning=capnpc failed, but continuing with rust-analyzer");
        }

        return Ok(());
    } else {
        tonic_build_result.expect("tonic_build command");
        capnpc_result.expect("schema compiler command");
    }

    Ok(())
}
