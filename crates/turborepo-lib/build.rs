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
        .compile(&["turbod.proto"], &["../../cli/internal/turbodprotocol"])?;
    Ok(())
}
