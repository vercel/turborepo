use super::*;

fn redirect(status: u16, location: &str) -> String {
    format!(
        "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: \
         close\r\n\r\n"
    )
}

fn client(source: &Fixture, targets: &[&Fixture]) -> Result<DownloadClient, Error> {
    let targets: Vec<_> = targets
        .iter()
        .map(|fixture| ApprovedOrigin::loopback_http_for_tests(&fixture.origin))
        .collect::<Result<_, _>>()?;
    Ok(source.client()?.with_redirect_origins(targets))
}

#[tokio::test]
async fn approved_hops_reach_digest_verified_final_bytes() -> TestResult {
    let target = Fixture::new(OK, "", Duration::ZERO).await?;
    let middle = Fixture::new(&redirect(302, &target.url()), "", Duration::ZERO).await?;
    for status in [301, 302, 303, 307, 308] {
        let source = Fixture::new(&redirect(status, &middle.url()), "", Duration::ZERO).await?;
        let client = client(&source, &[&middle, &target])?;
        let artifact = client
            .download_verified(
                &source.url(),
                ExpectedSha256::from_hex(ABC_SHA256)?,
                limits(3)?,
            )
            .await?;
        assert_eq!(artifact.as_bytes(), b"abc");
        assert_eq!(
            client.read_metadata(&source.url(), limits(3)?).await?,
            b"abc"
        );
        assert_eq!(source.hits(), 2);
    }
    let wrong = "a".repeat(64);
    let source = Fixture::new(&redirect(302, &target.url()), "", Duration::ZERO).await?;
    assert_eq!(
        client(&source, &[&target])?
            .download_verified(&source.url(), ExpectedSha256::from_hex(&wrong)?, limits(3)?)
            .await
            .err(),
        Some(Error::DigestMismatch)
    );
    // Byte limits still apply to the final response.
    assert_eq!(
        client(&source, &[&target])?
            .read_metadata(&source.url(), limits(2)?)
            .await,
        Err(Error::TooLarge)
    );
    Ok(())
}

#[tokio::test]
async fn unapproved_malformed_or_unbounded_redirects_are_rejected() -> TestResult {
    let target = Fixture::new(OK, "", Duration::ZERO).await?;
    let other = Fixture::new(OK, "", Duration::ZERO).await?;
    let port = Url::parse(&target.origin)?.port().ok_or("fixture port")?;
    for location in [
        target.url(),
        format!("http://user:secret@127.0.0.1:{port}/artifact"),
        format!("http://localhost:{port}/artifact"),
        target.url().replacen("http://", "https://", 1),
    ] {
        let source = Fixture::new(&redirect(302, &location), "", Duration::ZERO).await?;
        // Approving `other` never approves an unrelated or credentialed target.
        let client = client(&source, &[&other])?;
        assert_eq!(
            client.read_metadata(&source.url(), limits(3)?).await,
            Err(Error::RedirectRejected)
        );
        assert_eq!(source.hits(), 1);
    }
    let missing = "HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let source = Fixture::new(missing, "", Duration::ZERO).await?;
    assert_eq!(
        client(&source, &[&target])?
            .read_metadata(&source.url(), limits(3)?)
            .await,
        Err(Error::RedirectRejected)
    );
    // A relative self-redirect resolves against the current URL; only explicit
    // approval permits it, and then the hop limit ends the loop.
    let source = Fixture::new(&redirect(307, "/again"), "", Duration::ZERO).await?;
    assert_eq!(
        source
            .client()?
            .read_metadata(&source.url(), limits(3)?)
            .await,
        Err(Error::RedirectRejected)
    );
    assert_eq!(source.hits(), 1);
    let looping = client(&source, &[&source])?;
    assert_eq!(
        looping.read_metadata(&source.url(), limits(3)?).await,
        Err(Error::RedirectRejected)
    );
    assert_eq!(source.hits(), 1 + MAX_REDIRECTS + 1);
    assert_eq!(target.hits() + other.hits(), 0);
    Ok(())
}
