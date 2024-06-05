# Update-informer

[ci-badge]: https://github.com/mgrachev/update-informer/workflows/CI/badge.svg
[ci-url]: https://github.com/mgrachev/update-informer/actions
[crates-badge]: https://img.shields.io/crates/v/update-informer
[crates-url]: https://crates.io/crates/update-informer
[docs-badge]: https://img.shields.io/docsrs/update-informer
[docs-url]: https://docs.rs/update-informer
[codecov-badge]: https://codecov.io/gh/mgrachev/update-informer/branch/main/graph/badge.svg?token=A4XD1DGFGJ
[codecov-url]: https://codecov.io/gh/mgrachev/update-informer
[downloads-badge]: https://img.shields.io/crates/d/update-informer
[directories]: https://github.com/dirs-dev/directories-rs
[ureq]: https://github.com/algesten/ureq
[semver]: https://github.com/dtolnay/semver
[serde]: https://github.com/serde-rs/serde
[GitHub CLI application]: https://github.com/cli/cli/blob/trunk/internal/update/update.go
[npm]: https://github.com/npm/cli/blob/latest/lib/utils/update-notifier.js
[JavaScript library]: https://github.com/yeoman/update-notifier
[MIT]: https://choosealicense.com/licenses/mit
[git-cliff]: https://github.com/orhun/git-cliff
[dotenv-linter]: https://github.com/dotenv-linter/dotenv-linter
[update-informer]: https://evrone.com/update-informer?utm_source=github&utm_campaign=update-informer
[Evrone]: https://evrone.com/?utm_source=github&utm_campaign=update-informer
[turbo]: https://github.com/vercel/turbo
[fselect]: https://github.com/jhspetersson/fselect
[reqwest]: https://github.com/seanmonstar/reqwest
[isahc]: https://github.com/sagebind/isahc
[here]: https://github.com/mgrachev/update-informer/tree/main/examples

[![CI][ci-badge]][ci-url]
[![Version][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]
[![Codecov][codecov-badge]][codecov-url]
[![Downloads][downloads-badge]][crates-url]

<img align="right"
     alt="update-informer"
     src="https://raw.githubusercontent.com/mgrachev/update-informer/main/logo.svg?sanitize=true">

Update informer for CLI applications written in Rust 🦀

It checks for a new version on Crates.io, GitHub, Npm and PyPI 🚀

## Benefits

-   Support of **Crates.io**, **GitHub**, **Npm** and **PyPI**.
-   Configurable [check frequency](#interval) and [request timeout](#request-timeout).
-   [Caching](#caching) the results of checking updates.
-   Ability to implement your own [registry](#implementing-your-own-registry) or [http client](#using-your-own-http-client).
-   **Minimum dependencies** - only [directories], [semver], [serde] and an HTTP client ([ureq] or [reqwest]).

## Idea

The idea is actually not new. This feature has long been present in the [GitHub CLI application] and [npm].<br>
There is also a popular [JavaScript library].

## Usage

Add `update-informer` to `Cargo.toml`:

```toml
[dependencies]
update-informer = "1.1"
```

By default, `update-informer` can only check on Crates.io and uses [ureq] as a default HTTP client.
To enable support for other registries or change the HTTP client, use `features`:

```toml
[dependencies]
update-informer = { version = "1.1", default-features = false, features = ["github", "reqwest", "native-tls"] }
```

Available features:

| Name       | Type                | Default? |
| ---------- | ------------------- | -------- |
| crates     | Registry            | Yes      |
| github     | Registry            | No       |
| npm        | Registry            | No       |
| pypi       | Registry            | No       |
| [ureq]     | HTTP client         | Yes      |
| [reqwest]  | HTTP client         | No       |
| rustls-tls | HTTP client feature | Yes      |
| native-tls | HTTP client feature | No       |

## Checking for a new version

To check for a new version, use the `UpdateInformer::check_version` function.<br>
This function takes the project name and current version as well as registry:

```rust
use update_informer::{registry, Check};

let name = env!("CARGO_PKG_NAME");
let version = env!("CARGO_PKG_VERSION");
let informer = update_informer::new(registry::Crates, name, version);

if let Some(version) = informer.check_version().ok().flatten()  {
    println!("New version is available: {}", version);
}
```

More examples you can find [here].

## Interval

Note that the first check will start only after the interval has expired.
By default, the interval is **24 hours**, but you can change it:

```rust
use std::time::Duration;
use update_informer::{registry, Check};

const EVERY_HOUR: Duration = Duration::from_secs(60 * 60);

let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").interval(EVERY_HOUR);
informer.check_version(); // The check will start only after an hour
```

## Caching

By default, `update-informer` creates a file in the cache directory to avoid spam requests to the registry API.

In order not to cache requests, use a zero interval:

```rust
use std::time::Duration;
use update_informer::{registry, Check};

let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").interval(Duration::ZERO);
informer.check_version();
```

## Request timeout

You can also change the request timeout. By default, it is **5 seconds**:

```rust
use std::time::Duration;
use update_informer::{registry, Check};

const THIRTY_SECONDS: Duration = Duration::from_secs(30);

let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").timeout(THIRTY_SECONDS);
informer.check_version();
```

## Implementing your own registry

You can implement your own registry to check updates. For example:

```rust
use update_informer::{http_client::{GenericHttpClient, HttpClient}, registry, Check, Package, Registry, Result};

#[derive(serde::Deserialize)]
struct Response {
    version: String,
}

struct YourOwnRegistry;
impl Registry for YourOwnRegistry {
    const NAME: &'static str = "your_own_registry";

    fn get_latest_version<T: HttpClient>(http_client: GenericHttpClient<T>, pkg: &Package) -> Result<Option<String>> {
        let url = "https://turbo.build/api/binaries/version";
        let resp = http_client.get::<Response>(&url)?;

        Ok(Some(resp.version))
    }
}

let informer = update_informer::new(YourOwnRegistry, "turbo", "0.1.0");
informer.check_version();
```

## Using your own HTTP client

You can use your own HTTP client to check updates. For example, [isahc]:

```rust
use isahc::ReadResponseExt;
use std::time::Duration;
use serde::de::DeserializeOwned;
use update_informer::{http_client::{HeaderMap, HttpClient}, registry, Check};

struct YourOwnHttpClient;

impl HttpClient for YourOwnHttpClient {
    fn get<T: DeserializeOwned>(
        url: &str,
        _timeout: Duration,
        _headers: HeaderMap,
    ) -> update_informer::Result<T> {
        let json = isahc::get(url)?.json()?;
        Ok(json)
    }
}

let informer = update_informer::new(registry::Crates, "crate_name", "0.1.0").http_client(YourOwnHttpClient);
informer.check_version();
```

## Tests

In order not to check for updates in tests, you can use the `FakeUpdateInformer::check_version` function, which returns the desired version:

```rust
use update_informer::{registry, Check};

let name = "crate_name";
let version = "0.1.0";

#[cfg(not(test))]
let informer = update_informer::new(registry::Crates, name, version);

#[cfg(test)]
let informer = update_informer::fake(registry::Crates, name, version, "1.0.0");

if let Some(version) = informer.check_version().ok().flatten() {
    println!("New version is available: {}", version);
}
```

## Integration tests

To use the `FakeUpdateInformer::check_version` function in integration tests, you must first add the feature flag to `Cargo.toml`:

```toml
[features]
stub_check_version = []
```

Then use this feature flag in your code and integration tests:

```rust
use update_informer::{registry, Check};

let name = "crate_name";
let version = "0.1.0";

#[cfg(not(feature = "stub_check_version"))]
let informer = update_informer::new(registry::Crates, name, version);

#[cfg(feature = "stub_check_version")]
let informer = update_informer::fake(registry::Crates, name, version, "1.0.0");

informer.check_version();
```

## Users

-   [git-cliff]
-   [dotenv-linter]
-   [turbo]
-   [fselect]

## MSRV

Minimum Supported Rust Version: 1.56.1

## Sponsors

[update-informer] is created & supported by [Evrone]

## License

[MIT]
