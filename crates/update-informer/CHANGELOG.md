# Changelog

All notable changes to this project will be documented in this file.

## [v1.1.0](https://github.com/mgrachev/update-informer/releases/tag/v1.1.0) - 2023-06-27

### Features

-   Add `rustls-tls` and `native-tls` features ([#111](https://github.com/mgrachev/update-informer/pull/111))
-   Add `User-Agent` header for GitHub registry ([#114](https://github.com/mgrachev/update-informer/pull/114))

### Miscellaneous Tasks

-   Fix typo in README ([#112](https://github.com/mgrachev/update-informer/pull/112))
-   Update exempt labels for stale action ([#107](https://github.com/mgrachev/update-informer/pull/107))

### Update dependencies

-   Bump Swatinem/rust-cache from 2.4.0 to 2.5.0 ([#108](https://github.com/mgrachev/update-informer/pull/108))
-   Bump Swatinem/rust-cache from 2.3.0 to 2.4.0 ([#106](https://github.com/mgrachev/update-informer/pull/106))
-   Bump lycheeverse/lychee-action from 1.7.0 to 1.8.0 ([#105](https://github.com/mgrachev/update-informer/pull/105))
-   Bump Swatinem/rust-cache from 2.2.1 to 2.3.0 ([#104](https://github.com/mgrachev/update-informer/pull/104))

## [v1.0.0](https://github.com/mgrachev/update-informer/releases/tag/v1.0.0) - 2023-05-08

### Features

-   Add `npm` registry support ([#80](https://github.com/mgrachev/update-informer/pull/80))
-   Add `reqwest` crate support ([#81](https://github.com/mgrachev/update-informer/pull/81))
-   Add ability to use your own http client ([#83](https://github.com/mgrachev/update-informer/pull/83))
-   Use undefined http client if no other is selected ([#89](https://github.com/mgrachev/update-informer/pull/89))
-   Add `http_client` method for `FakeUpdateInformer` ([#87](https://github.com/mgrachev/update-informer/pull/87))

### CI

-   Check PR name instead of commits ([#85](https://github.com/mgrachev/update-informer/pull/85))

### Miscellaneous Tasks

-   Add example declarations for examples that have required features ([#90](https://github.com/mgrachev/update-informer/pull/90))
-   Add more examples ([#88](https://github.com/mgrachev/update-informer/pull/88))
-   Get rid of `orhun/git-cliff-action` ([#78](https://github.com/mgrachev/update-informer/pull/78))

### Refactor

-   Replace `Option<(&'a str, &'a str)>` with `HeaderMap` ([#101](https://github.com/mgrachev/update-informer/pull/101), [#102](https://github.com/mgrachev/update-informer/pull/102))
-   Change trait name `SendRequest` -> `HttpClient` ([#92](https://github.com/mgrachev/update-informer/pull/92))
-   Remove deprecated `FakeUpdateInformer::new` function ([#86](https://github.com/mgrachev/update-informer/pull/86))
-   Move current_version to package ([#82](https://github.com/mgrachev/update-informer/pull/82))
-   Remove deprecated function ([#79](https://github.com/mgrachev/update-informer/pull/79))

### Update dependencies

-   Update directories requirement from 4.0 to 5.0 ([#98](https://github.com/mgrachev/update-informer/pull/98))
-   Update mockito requirement from 0.31 to 0.32 ([#91](https://github.com/mgrachev/update-informer/pull/91))
-   Bump actions/stale from 7 to 8 ([#99](https://github.com/mgrachev/update-informer/pull/99))
-   Bump lycheeverse/lychee-action from 1.6.1 to 1.7.0 ([#100](https://github.com/mgrachev/update-informer/pull/100))
-   Bump Swatinem/rust-cache from 2.2.0 to 2.2.1 ([#95](https://github.com/mgrachev/update-informer/pull/95))
-   Bump lycheeverse/lychee-action from 1.5.4 to 1.6.1 ([#94](https://github.com/mgrachev/update-informer/pull/94))
-   Bump dprint/check from 2.1 to 2.2 ([#84](https://github.com/mgrachev/update-informer/pull/84))
-   Bump actions/stale from 6 to 7 ([#76](https://github.com/mgrachev/update-informer/pull/76))

## [v0.6.0](https://github.com/mgrachev/update-informer/releases/tag/v0.6.0) - 2022-12-08

### 🚀 Added

-   Support current version ([#72](https://github.com/mgrachev/update-informer/pull/72))
-   Add cargo-sort ([#51](https://github.com/mgrachev/update-informer/pull/51))
-   Add list of users
-   Add `dprint` to check formatting ([#42](https://github.com/mgrachev/update-informer/pull/42))
-   Add action to check links + ci optimization ([#38](https://github.com/mgrachev/update-informer/pull/38))

### ⚙️ Changed

-   Bump wagoid/commitlint-github-action from 5.2.2 to 5.3.0 ([#71](https://github.com/mgrachev/update-informer/pull/71))
-   Bump lycheeverse/lychee-action from 1.5.3 to 1.5.4 ([#69](https://github.com/mgrachev/update-informer/pull/69))
-   Bump Swatinem/rust-cache from 2.1.0 to 2.2.0 ([#70](https://github.com/mgrachev/update-informer/pull/70))
-   Bump lycheeverse/lychee-action from 1.5.2 to 1.5.3 ([#68](https://github.com/mgrachev/update-informer/pull/68))
-   Bump lycheeverse/lychee-action from 1.5.1 to 1.5.2 ([#64](https://github.com/mgrachev/update-informer/pull/64))
-   Bump Swatinem/rust-cache from 2.0.1 to 2.1.0 ([#67](https://github.com/mgrachev/update-informer/pull/67))
-   Fix clippy warnings ([#66](https://github.com/mgrachev/update-informer/pull/66))
-   Bump wagoid/commitlint-github-action from 5.2.0 to 5.2.2 ([#63](https://github.com/mgrachev/update-informer/pull/63))
-   Bump Swatinem/rust-cache from 2.0.0 to 2.0.1 ([#62](https://github.com/mgrachev/update-informer/pull/62))
-   Bump wagoid/commitlint-github-action from 5.1.2 to 5.2.0 ([#60](https://github.com/mgrachev/update-informer/pull/60))
-   Replace `actions-rs/toolchain` with `dtolnay/rust-toolchain` ([#61](https://github.com/mgrachev/update-informer/pull/61))
-   Bump wagoid/commitlint-github-action from 5.0.2 to 5.1.2 ([#59](https://github.com/mgrachev/update-informer/pull/59))
-   Bump actions/stale from 5 to 6 ([#58](https://github.com/mgrachev/update-informer/pull/58))
-   Generate a changelog and update release process ([#57](https://github.com/mgrachev/update-informer/pull/57))
-   Update readme ([#56](https://github.com/mgrachev/update-informer/pull/56))
-   Bump lycheeverse/lychee-action from 1.5.0 to 1.5.1 ([#55](https://github.com/mgrachev/update-informer/pull/55))
-   Bump Swatinem/rust-cache from 1.4.0 to 2.0.0 ([#53](https://github.com/mgrachev/update-informer/pull/53))
-   Bump dprint/check from 2.0 to 2.1 ([#54](https://github.com/mgrachev/update-informer/pull/54))
-   Bump wagoid/commitlint-github-action from 5.0.1 to 5.0.2 ([#52](https://github.com/mgrachev/update-informer/pull/52))
-   Bump wagoid/commitlint-github-action from 4.1.11 to 5.0.1 ([#50](https://github.com/mgrachev/update-informer/pull/50))
-   Bump lycheeverse/lychee-action from 1.4.1 to 1.5.0 ([#48](https://github.com/mgrachev/update-informer/pull/48))
-   Bump actions/stale from 4 to 5 ([#45](https://github.com/mgrachev/update-informer/pull/45))
-   Bump Swatinem/rust-cache from 1.3.0 to 1.4.0 ([#44](https://github.com/mgrachev/update-informer/pull/44))
-   Bump wagoid/commitlint-github-action from 4.1.10 to 4.1.11 ([#43](https://github.com/mgrachev/update-informer/pull/43))
-   Bump codecov/codecov-action from 2.1.0 to 3 ([#41](https://github.com/mgrachev/update-informer/pull/41))
-   Bump actions-rs/toolchain from 1.0.6 to 1.0.7 ([#39](https://github.com/mgrachev/update-informer/pull/39))
-   Bump wagoid/commitlint-github-action from 4.1.9 to 4.1.10 ([#40](https://github.com/mgrachev/update-informer/pull/40))

## [v0.5.0](https://github.com/mgrachev/update-informer/releases/tag/v0.5.0) - 2022-03-24

### 🚀 Added

-   Add ability to implement your own registry ([#37](https://github.com/mgrachev/update-informer/pull/37))
-   Add `stale` action ([#33](https://github.com/mgrachev/update-informer/pull/33))
-   Add dependabot ([#28](https://github.com/mgrachev/update-informer/pull/28))

### ⚙️ Changed

-   Bump actions/cache from 2 to 3 ([#36](https://github.com/mgrachev/update-informer/pull/36))
-   Update `stale` action schedule
-   Update mockito requirement from 0.30.0 to 0.31.0 ([#32](https://github.com/mgrachev/update-informer/pull/32))
-   Bump actions/checkout from 1 to 3 ([#31](https://github.com/mgrachev/update-informer/pull/31))
-   Bump wagoid/commitlint-github-action from 2 to 4.1.9 ([#29](https://github.com/mgrachev/update-informer/pull/29))
-   Bump codecov/codecov-action from 1 to 2.1.0 ([#30](https://github.com/mgrachev/update-informer/pull/30))
-   Update dependabot config
-   Update `stale` schedule
-   Update `stale` schedule ([#35](https://github.com/mgrachev/update-informer/pull/35))
-   Update `stale` action schedule ([#34](https://github.com/mgrachev/update-informer/pull/34))

## [v0.4.0](https://github.com/mgrachev/update-informer/releases/tag/v0.4.0) - 2022-02-21

### 🚀 Added

-   Add ability to not use cache files ([#27](https://github.com/mgrachev/update-informer/pull/27))

## [v0.3.0](https://github.com/mgrachev/update-informer/releases/tag/v0.3.0) - 2022-02-19

### 🚀 Added

-   Add cargo features ([#26](https://github.com/mgrachev/update-informer/pull/26))
-   Add configurable request timeout and interval ([#24](https://github.com/mgrachev/update-informer/pull/24))
-   Add open collective
-   Add more examples ([#23](https://github.com/mgrachev/update-informer/pull/23))
-   Add logo ([#19](https://github.com/mgrachev/update-informer/pull/19))

### ⚙️ Changed

-   Better cache directory naming scheme ([#21](https://github.com/mgrachev/update-informer/pull/21))
-   PyPI support ([#16](https://github.com/mgrachev/update-informer/pull/16))
-   Update readme
-   Set up code coverage ([#15](https://github.com/mgrachev/update-informer/pull/15))

## [v0.2.0](https://github.com/mgrachev/update-informer/releases/tag/v0.2.0) - 2022-01-05

### 🚀 Added

-   Add `UpdateInformer` and `FakeUpdateInformer` structs for convenient use ([#14](https://github.com/mgrachev/update-informer/pull/14))

## [v0.1.0](https://github.com/mgrachev/update-informer/releases/tag/v0.1.0) - 2021-12-30

### 🚀 Added

-   Add `stub_check_version` function and update docs ([#13](https://github.com/mgrachev/update-informer/pull/13))
-   Add documentation and update examples ([#12](https://github.com/mgrachev/update-informer/pull/12))
-   Add tests for registries: crates.io and github ([#9](https://github.com/mgrachev/update-informer/pull/9))
-   Add main files

### ⚙️ Changed

-   Save latest version to file and add interval check ([#11](https://github.com/mgrachev/update-informer/pull/11))
-   Set up ci/cd ([#10](https://github.com/mgrachev/update-informer/pull/10))
-   Check updates on github ([#8](https://github.com/mgrachev/update-informer/pull/8))
-   Check version on crates.io ([#1](https://github.com/mgrachev/update-informer/pull/1))
-   Initial commit
