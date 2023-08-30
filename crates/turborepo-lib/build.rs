#[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
fn check_tls_config() {}
#[cfg(not(any(feature = "native-tls", feature = "rustls-tls")))]
fn check_tls_config() {
    panic!("You must enable one of the TLS features: native-tls or rustls-tls");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    check_tls_config();

    tonic_build::configure()
        .build_server(true)
        .file_descriptor_set_path("src/daemon/file_descriptor_set.bin")
        .compile(&["turbod.proto"], &["../../cli/internal/turbodprotocol"])?;

    capnpc::CompilerCommand::new()
        .file("./src/hash/proto.capnp")
        .import_path("./src/hash/std") // we need to include the 'stdlib' for capnp-go
        .default_parent_module(vec!["hash".to_string()])
        .run()
        .expect("schema compiler command");

    Ok(())
}
