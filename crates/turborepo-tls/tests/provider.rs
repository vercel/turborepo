use std::{
    process::Command,
    sync::{Arc, Barrier},
};

use rustls::crypto::CryptoProvider;

#[test]
fn installation_races_and_preserves_existing_provider() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(mode) = std::env::var("TURBO_TLS_PROVIDER_FIXTURE") else {
        for mode in ["bootstrap", "already-installed"] {
            assert!(
                Command::new(std::env::current_exe()?)
                    .env("TURBO_TLS_PROVIDER_FIXTURE", mode)
                    .arg("--exact")
                    .arg("installation_races_and_preserves_existing_provider")
                    .status()?
                    .success()
            );
        }
        return Ok(());
    };
    assert!(CryptoProvider::get_default().is_none());
    if mode == "already-installed" {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| "fixture install")?;
    }
    let before = CryptoProvider::get_default().cloned();
    let barrier = Arc::new(Barrier::new(16));
    let threads: Vec<_> = (0..16)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                turborepo_tls::ensure_crypto_provider();
                CryptoProvider::get_default().cloned()
            })
        })
        .collect();
    let installed = threads
        .into_iter()
        .map(|thread| thread.join().map_err(|_| "provider panicked"))
        .collect::<Result<Vec<_>, _>>()?;
    let provider = CryptoProvider::get_default().ok_or("provider missing")?;
    assert!(installed.iter().all(|value| {
        value
            .as_ref()
            .is_some_and(|value| Arc::ptr_eq(value, provider))
    }));
    if let Some(before) = before {
        assert!(Arc::ptr_eq(&before, provider));
    } else {
        let ring = rustls::crypto::ring::default_provider();
        assert_eq!(
            provider.signature_verification_algorithms.all.len(),
            ring.signature_verification_algorithms.all.len() + 3
        );
        assert_eq!(provider.cipher_suites, ring.cipher_suites);
    }
    Ok(())
}
