use std::{sync::Arc, time::Duration};

use rustls::{
    CertificateError, Error, RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
};

// Public certificates captured on 2026-10-20 from
// https://emudhra.we-have-a-problem.com/{chains.zip,roots.json}.
#[test]
fn rejects_compromised_emudhra_intermediates_with_trusted_roots()
-> Result<(), Box<dyn std::error::Error>> {
    turborepo_tls::ensure_crypto_provider();
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(include_bytes!("fixtures/emudhra-roots.pem")) {
        roots.add(cert?)?;
    }
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        CryptoProvider::get_default()
            .ok_or("provider missing")?
            .clone(),
    )
    .build()?;
    // Keep validation inside the captured certificates' validity windows.
    let now = UnixTime::since_unix_epoch(Duration::from_secs(1_792_497_600));
    for (host, pem) in [
        (
            "dv-g3b-emudhra.we-have-a-problem.com",
            include_bytes!("fixtures/dv-g3b-emudhra.we-have-a-problem.com.pem").as_slice(),
        ),
        (
            "dv-g3b-aaa-emudhra.we-have-a-problem.com",
            include_bytes!("fixtures/dv-g3b-aaa-emudhra.we-have-a-problem.com.pem").as_slice(),
        ),
    ] {
        let chain = CertificateDer::pem_slice_iter(pem).collect::<Result<Vec<_>, _>>()?;
        let error = verifier
            .verify_server_cert(
                &chain[0],
                &chain[1..],
                &ServerName::try_from(host)?,
                &[],
                now,
            )
            .expect_err("compromised intermediate must be rejected even with its root trusted");
        assert!(
            matches!(error, Error::InvalidCertificate(CertificateError::Other(_))),
            "expected explicit distrust for {host}, got {error:?}"
        );
    }
    Ok(())
}
