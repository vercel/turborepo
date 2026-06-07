//! Process-wide rustls crypto provider configuration.
//!
//! rustls' default `ring` crypto provider has no support for P-521
//! (`secp521r1`) ECDSA at all. As a result, any TLS certificate chain that
//! contains a P-521 issuer is rejected during verification with
//! `UnsupportedSignatureAlgorithmForPublicKeyContext`, even though curl, Node,
//! and the OS-native TLS stacks happily accept the same chain. This shows up in
//! the wild against remote caches behind Cloudflare Zero Trust / WARP TLS
//! inspection, where the inspecting CA is a P-521 ECDSA certificate (see
//! <https://github.com/vercel/turborepo/issues/13035>).
//!
//! `ring` cannot verify P-521 signatures, so we install a process-wide
//! `CryptoProvider` that keeps `ring` for everything it already does and only
//! borrows the P-521 certificate-signature verification algorithms from
//! `aws-lc-rs`. This is the narrowest change that fixes the chain while leaving
//! cipher suites, key exchange, and handshake signing untouched.

use std::sync::Once;

use rustls::{
    SignatureScheme,
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms},
    pki_types::SignatureVerificationAlgorithm,
};

static INSTALL: Once = Once::new();

/// Ensures a rustls `CryptoProvider` capable of verifying P-521 certificate
/// chains is installed as the process-wide default.
///
/// reqwest (and any other rustls consumer in the process) picks up the default
/// provider via `CryptoProvider::get_default()`, so installing it once here
/// fixes every TLS client we build. The work happens at most once and is cheap
/// to call repeatedly. If another provider has already been installed we leave
/// it in place rather than fighting over the global default.
pub(crate) fn ensure_crypto_provider() {
    INSTALL.call_once(|| {
        if CryptoProvider::get_default().is_some() {
            return;
        }
        // Ignore the error: the only failure mode is that another thread
        // installed a default between the check above and here, which is fine.
        let _ = provider_with_p521().install_default();
    });
}

/// Builds a `ring`-based provider whose certificate signature verification
/// algorithms are augmented with the aws-lc-rs P-521 implementations.
fn provider_with_p521() -> CryptoProvider {
    let base = rustls::crypto::ring::default_provider();
    CryptoProvider {
        signature_verification_algorithms: signature_algorithms_with_p521(
            base.signature_verification_algorithms,
        ),
        ..base
    }
}

/// Returns `ring`'s supported signature algorithms with the three P-521 ECDSA
/// verification algorithms (SHA-256/384/512) added. `ring` never supplies P-521
/// algorithms, so these are purely additive.
fn signature_algorithms_with_p521(
    ring_algs: WebPkiSupportedAlgorithms,
) -> WebPkiSupportedAlgorithms {
    const P521_ALGS: &[&dyn SignatureVerificationAlgorithm] = &[
        webpki::aws_lc_rs::ECDSA_P521_SHA256,
        webpki::aws_lc_rs::ECDSA_P521_SHA384,
        webpki::aws_lc_rs::ECDSA_P521_SHA512,
    ];

    // `all` is used to verify signatures within the certificate chain (the
    // P-521 issuer case from the bug report).
    let mut all: Vec<&'static dyn SignatureVerificationAlgorithm> = ring_algs.all.to_vec();
    all.extend_from_slice(P521_ALGS);

    // `mapping` is used to verify the server's handshake signature, keyed by the
    // TLS `SignatureScheme`. Add P-521 so endpoints presenting a P-521 leaf can
    // complete the handshake too. ECDSA_NISTP521_SHA512 is the only ECDSA P-521
    // scheme defined by TLS.
    let mut mapping = ring_algs.mapping.to_vec();
    mapping.push((
        SignatureScheme::ECDSA_NISTP521_SHA512,
        const { &[webpki::aws_lc_rs::ECDSA_P521_SHA512] },
    ));

    WebPkiSupportedAlgorithms {
        all: Box::leak(all.into_boxed_slice()),
        mapping: Box::leak(mapping.into_boxed_slice()),
    }
}

#[cfg(test)]
mod test {
    use rustls::SignatureScheme;

    use super::signature_algorithms_with_p521;

    /// The augmentation is purely additive: every algorithm `ring` already
    /// supported is still present, plus the three P-521 algorithms and the
    /// P-521 handshake mapping.
    #[test]
    fn augmentation_is_additive() {
        let ring_algs = rustls::crypto::ring::default_provider().signature_verification_algorithms;
        let augmented = signature_algorithms_with_p521(ring_algs);

        assert_eq!(augmented.all.len(), ring_algs.all.len() + 3);
        assert_eq!(augmented.mapping.len(), ring_algs.mapping.len() + 1);
        assert!(
            augmented
                .mapping
                .iter()
                .any(|(scheme, _)| *scheme == SignatureScheme::ECDSA_NISTP521_SHA512)
        );
    }
}
