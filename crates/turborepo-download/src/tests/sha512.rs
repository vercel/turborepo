use super::*;

const ABC_SHA512: &str = concat!(
    "ddaf35a193617abacc417349ae20413112",
    "e6fa4e89a97ea20a9eeee64b55d39a219",
    "2992a274fc1a836ba3c23a3feebbd454d",
    "4423643ce80e2a9ac94fa54ca49f"
);

fn digest() -> Result<ExpectedSha512, Error> {
    ExpectedSha512::from_hex(ABC_SHA512)
}

#[test]
fn mandatory_hex_and_verified_identity() -> TestResult {
    assert!(matches!(
        ExpectedSha512::from_hex(""),
        Err(Error::MissingSha512Digest)
    ));
    for bad in [
        "abc".into(),
        "0".repeat(127),
        "0".repeat(129),
        "g".repeat(128),
        "é".repeat(64),
        format!("sha512.{ABC_SHA512}"),
    ] {
        assert!(matches!(
            ExpectedSha512::from_hex(&bad),
            Err(Error::InvalidSha512Digest)
        ));
    }
    for sha256 in [None, Some(ExpectedSha256::from_hex(ABC_SHA256)?)] {
        let artifact = VerifiedArtifact::verify_bytes_sha512(
            b"abc".to_vec(),
            ExpectedSha512::from_hex(&ABC_SHA512.to_uppercase())?,
            sha256,
        )?;
        assert_eq!(artifact.sha256_hex(), ABC_SHA256);
        assert_eq!(artifact.as_bytes(), b"abc");
        assert_eq!(artifact.into_bytes(), b"abc");
    }
    let empty = hex::encode(Sha512::digest([]));
    let artifact =
        VerifiedArtifact::verify_bytes_sha512(Vec::new(), ExpectedSha512::from_hex(&empty)?, None)?;
    assert!(artifact.as_bytes().is_empty());
    Ok(())
}

#[test]
fn mismatch_releases_no_bytes_and_sha512_precedes_additional_sha256() -> TestResult {
    let bad512 = ExpectedSha512::from_hex(&"0".repeat(128))?;
    let bad256 = ExpectedSha256::from_hex(&"0".repeat(64))?;
    for (sha512, sha256, expected) in [
        (bad512, None, Error::Sha512DigestMismatch),
        (bad512, Some(bad256), Error::Sha512DigestMismatch),
        (digest()?, Some(bad256), Error::DigestMismatch),
    ] {
        let error = VerifiedArtifact::verify_bytes_sha512(b"abc".to_vec(), sha512, sha256)
            .err()
            .ok_or("mismatch required")?;
        assert_eq!(error, expected);
        let diagnostic = format!("{error} {error:?}");
        assert!(!diagnostic.contains(ABC_SHA512));
        assert!(!diagnostic.contains(ABC_SHA256));
        assert!(StdError::source(&error).is_none());
    }
    assert!(matches!(
        VerifiedArtifact::verify_bytes_sha512(b"abd".to_vec(), digest()?, None),
        Err(Error::Sha512DigestMismatch)
    ));
    Ok(())
}

#[tokio::test]
async fn bounded_download_reuses_verified_artifact_and_checks_both_pins() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    let client = fixture.client()?;
    for sha256 in [None, Some(ExpectedSha256::from_hex(ABC_SHA256)?)] {
        let artifact = client
            .download_verified_sha512(&fixture.url(), digest()?, sha256, limits(3)?)
            .await?;
        assert_eq!(artifact.sha256_hex(), ABC_SHA256);
        assert_eq!(artifact.as_bytes(), b"abc");
    }
    let bad512 = ExpectedSha512::from_hex(&"0".repeat(128))?;
    let bad256 = ExpectedSha256::from_hex(&"0".repeat(64))?;
    for (sha512, sha256, expected) in [
        (bad512, Some(bad256), Error::Sha512DigestMismatch),
        (digest()?, Some(bad256), Error::DigestMismatch),
    ] {
        let error = client
            .download_verified_sha512(&fixture.url(), sha512, sha256, limits(3)?)
            .await
            .err()
            .ok_or("mismatch required")?;
        assert_eq!(error, expected);
        assert!(StdError::source(&error).is_none());
    }
    assert!(matches!(
        client
            .download_verified_sha512(&fixture.url(), digest()?, None, limits(2)?)
            .await,
        Err(Error::TooLarge)
    ));
    Ok(())
}

#[tokio::test]
async fn chunked_completion_truncation_and_total_deadline_are_unchanged() -> TestResult {
    let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    for (end, gap, timeout, failure) in [
        ("0\r\n\r\n", 0, 2000, None),
        ("", 0, 2000, Some(Error::RequestFailed)),
        ("0\r\n\r\n", 200, 300, Some(Error::TimedOut)),
    ] {
        let fixture = Fixture::parts(&[
            (header, Duration::ZERO),
            ("1\r\na\r\n", Duration::ZERO),
            ("1\r\nb\r\n", Duration::from_millis(gap)),
            ("1\r\nc\r\n", Duration::from_millis(gap)),
            (end, Duration::ZERO),
        ])
        .await?;
        let result = fixture
            .client()?
            .download_verified_sha512(
                &fixture.url(),
                digest()?,
                Some(ExpectedSha256::from_hex(ABC_SHA256)?),
                Limits::new(3, Duration::from_millis(timeout))?,
            )
            .await
            .map(VerifiedArtifact::into_bytes);
        assert_eq!(result, failure.map_or(Ok(b"abc".to_vec()), Err));
    }
    let oversized = Fixture::parts(&[
        (header, Duration::ZERO),
        ("4\r\nabcd\r\n0\r\n\r\n", Duration::ZERO),
    ])
    .await?;
    assert!(matches!(
        oversized
            .client()?
            .download_verified_sha512(&oversized.url(), digest()?, None, limits(3)?)
            .await,
        Err(Error::TooLarge)
    ));
    Ok(())
}

#[tokio::test]
async fn origins_credentials_and_redirects_do_not_gain_an_escape_hatch() -> TestResult {
    let target = Fixture::new(OK, "", Duration::ZERO).await?;
    let denied = DownloadClient::with_http_builder(http_fixture_builder(), [])?;
    assert!(matches!(
        denied
            .download_verified_sha512(&target.url(), digest()?, None, limits(3)?)
            .await,
        Err(Error::UnapprovedOrigin)
    ));
    let credentials = target.url().replacen("://", "://user:private-password@", 1);
    assert!(matches!(
        target
            .client()?
            .download_verified_sha512(&credentials, digest()?, None, limits(3)?)
            .await,
        Err(Error::UrlCredentials)
    ));
    for status in [301, 302, 303, 307, 308] {
        let response = format!(
            "HTTP/1.1 {status} Redirect\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
            target.url()
        );
        let source = Fixture::new(&response, "", Duration::ZERO).await?;
        let client = DownloadClient::with_http_builder(
            http_fixture_builder().redirect(Policy::limited(10)),
            [
                ApprovedOrigin::loopback_http_for_tests(&source.origin)?,
                ApprovedOrigin::loopback_http_for_tests(&target.origin)?,
            ],
        )?;
        assert!(matches!(
            client
                .download_verified_sha512(&source.url(), digest()?, None, limits(3)?)
                .await,
            Err(Error::RedirectRejected)
        ));
    }
    assert_eq!(target.hits(), 0);
    Ok(())
}
