#[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
fn check_tls_config() {}
#[cfg(not(any(feature = "native-tls", feature = "rustls-tls")))]
fn check_tls_config() {
    panic!("You must enable one of the TLS features: native-tls or rustls-tls");
}

fn main() {
    check_tls_config();
}
