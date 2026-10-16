//! `with_https_redirects` over real TLS: any HTTPS origin for digest-verified
//! bytes, never a downgrade, and no state carried between hops.
use super::*;

fn hop(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Redirect\r\nLocation: {location}\r\nSet-Cookie: \
         session=secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

async fn client(source: &Local) -> Result<DownloadClient, Box<dyn StdError>> {
    let approved = ApprovedOrigin::https(&source.https_origin())?;
    Ok(
        DownloadClient::with_http_builder(custom_root_builder()?, [approved])?
            .with_https_redirects(),
    )
}

#[tokio::test]
async fn verified_downloads_follow_https_redirects_to_unapproved_origins() -> TestResult {
    let target = Local::listen(Some(SERVER), None, OK).await?;
    let location = format!("{}/cdn/artifact", target.https_origin());
    let source = Local::listen(Some(SERVER), None, &hop(&location)).await?;
    let client = client(&source).await?;
    let url = format!("{}/release/artifact", source.https_origin());
    let digest = || ExpectedSha256::from_hex(ABC_SHA256);
    assert_eq!(
        client
            .download_verified(&url, digest()?, limits(3)?)
            .await?
            .as_bytes(),
        b"abc"
    );
    // The digest still binds the final bytes.
    let wrong = ExpectedSha256::from_hex(&"0".repeat(64))?;
    assert_eq!(
        client
            .download_verified(&url, wrong, limits(3)?)
            .await
            .err(),
        Some(Error::DigestMismatch)
    );
    // Unverified metadata never follows an unapproved redirect.
    assert_eq!(
        client.read_metadata(&url, limits(3)?).await,
        Err(Error::RedirectRejected)
    );
    let requests = target.requests()?;
    assert_eq!(requests.len(), 2);
    for request in requests {
        let request = request.to_ascii_lowercase();
        assert!(request.starts_with("get /cdn/artifact "), "{request}");
        for header in ["cookie", "authorization", "referer"] {
            assert!(!request.contains(&format!("\r\n{header}:")), "{request}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn https_redirects_never_downgrade_or_carry_credentials() -> TestResult {
    let plain = Local::listen(None, None, OK).await?;
    let port = plain.addr.port();
    for location in [
        format!("{}/artifact", plain.http_origin()),
        format!("http://localhost:{port}/artifact"),
        format!("https://user:secret@localhost:{port}/artifact"),
    ] {
        let source = Local::listen(Some(SERVER), None, &hop(&location)).await?;
        let url = format!("{}/artifact", source.https_origin());
        let digest = ExpectedSha256::from_hex(ABC_SHA256)?;
        assert_eq!(
            client(&source)
                .await?
                .download_verified(&url, digest, limits(3)?)
                .await
                .err(),
            Some(Error::RedirectRejected),
            "{location}"
        );
        assert_eq!(source.requests()?.len(), 1);
    }
    assert!(plain.requests()?.is_empty());
    Ok(())
}
