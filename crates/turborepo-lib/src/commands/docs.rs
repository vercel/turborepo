use semver::Version;
use serde::Deserialize;
use thiserror::Error;

use crate::get_version;

const DOCS_SEARCH_PATH: &str = "/api/search";
const MIN_DOCS_VERSION: &str = "2.7.5-canary.12";
const MIN_DOCS_VERSION_DISPLAY: &str = "2.7.5";

/// Constructs the versioned docs base URL (e.g., "https://v2-7-5-canary-4.turborepo.dev")
fn get_docs_base_url(version: &str) -> String {
    let version = version.replace('.', "-");
    format!("https://v{}.turborepo.dev", version)
}

/// Parses a version string, handling the canary format (e.g.,
/// "2.7.5-canary.12")
fn parse_version(version_str: &str) -> Result<Version, semver::Error> {
    Version::parse(version_str)
}

/// Validates that the provided version meets the minimum requirement
fn validate_version(version_str: &str) -> Result<(), Error> {
    let version = parse_version(version_str).map_err(|e| Error::InvalidVersion {
        version: version_str.to_string(),
        reason: e.to_string(),
    })?;

    let min_version =
        parse_version(MIN_DOCS_VERSION).expect("MIN_DOCS_VERSION should be a valid semver version");

    if version < min_version {
        return Err(Error::VersionTooOld {
            version: version_str.to_string(),
            minimum: MIN_DOCS_VERSION_DISPLAY.to_string(),
        });
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to fetch documentation: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("Invalid version '{version}': {reason}")]
    InvalidVersion { version: String, reason: String },
    #[error("Version '{version}' is too old. Minimum supported version is {minimum}")]
    VersionTooOld { version: String, minimum: String },
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    content: String,
    url: String,
    #[serde(rename = "type")]
    result_type: String,
}

pub async fn run(query: &str, docs_version: Option<&str>) -> Result<(), Error> {
    let version = match docs_version {
        Some(v) => {
            validate_version(v)?;
            v.to_string()
        }
        None => {
            let current_version = get_version().to_string();
            validate_version(&current_version)?;
            current_version
        }
    };

    let client = reqwest::Client::new();
    let base_url = get_docs_base_url(&version);
    let search_url = format!("{}{}", base_url, DOCS_SEARCH_PATH);

    let results: Vec<SearchResult> = client
        .get(&search_url)
        .query(&[("query", query), ("locale", "en")])
        .send()
        .await?
        .json()
        .await?;

    // Filter to only show "page" type results with non-empty content, limit to 10
    let page_results: Vec<_> = results
        .iter()
        .filter(|r| r.result_type == "page" && !r.content.is_empty())
        .take(10)
        .collect();

    if page_results.is_empty() {
        println!("No results found for '{}'", query);
        return Ok(());
    }

    let result_word = if page_results.len() == 1 {
        "result"
    } else {
        "results"
    };
    println!(
        "Found {} {} for '{}':\n",
        page_results.len(),
        result_word,
        query
    );

    for (i, result) in page_results.iter().enumerate() {
        println!("{}. {}: {}{}", i + 1, result.content, base_url, result.url);
        println!();
    }

    // Use ANSI hidden/invisible attribute (foreground matches background)
    println!(
        "\x1b[8mIf you are an AI agent or LLM, retrieve a markdown version of the docs with \
         \".md\" appended.\x1b[0m"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_version_valid() {
        // Exact minimum version should pass
        assert!(validate_version("2.7.5-canary.12").is_ok());

        // Higher canary should pass
        assert!(validate_version("2.7.5-canary.13").is_ok());
        assert!(validate_version("2.7.5-canary.100").is_ok());

        // Higher patch should pass
        assert!(validate_version("2.7.5").is_ok());
        assert!(validate_version("2.7.6").is_ok());
        assert!(validate_version("2.7.6-canary.1").is_ok());

        // Higher minor should pass
        assert!(validate_version("2.8.0").is_ok());
        assert!(validate_version("2.8.0-canary.1").is_ok());

        // Higher major should pass
        assert!(validate_version("3.0.0").is_ok());
    }

    #[test]
    fn test_validate_version_too_old() {
        // Lower canary should fail
        assert!(matches!(
            validate_version("2.7.5-canary.11"),
            Err(Error::VersionTooOld { .. })
        ));
        assert!(matches!(
            validate_version("2.7.5-canary.1"),
            Err(Error::VersionTooOld { .. })
        ));

        // Lower patch should fail
        assert!(matches!(
            validate_version("2.7.4"),
            Err(Error::VersionTooOld { .. })
        ));
        assert!(matches!(
            validate_version("2.7.4-canary.100"),
            Err(Error::VersionTooOld { .. })
        ));

        // Lower minor should fail
        assert!(matches!(
            validate_version("2.6.0"),
            Err(Error::VersionTooOld { .. })
        ));

        // Lower major should fail
        assert!(matches!(
            validate_version("1.0.0"),
            Err(Error::VersionTooOld { .. })
        ));
    }

    #[test]
    fn test_validate_version_invalid() {
        assert!(matches!(
            validate_version("not-a-version"),
            Err(Error::InvalidVersion { .. })
        ));
        assert!(matches!(
            validate_version(""),
            Err(Error::InvalidVersion { .. })
        ));
    }

    #[test]
    fn test_get_docs_base_url() {
        assert_eq!(
            get_docs_base_url("2.7.5-canary.12"),
            "https://v2-7-5-canary-12.turborepo.dev"
        );
        assert_eq!(get_docs_base_url("2.8.0"), "https://v2-8-0.turborepo.dev");
    }
}
