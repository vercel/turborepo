## Rust workflows

### cargo-install

- [taiki-e/install-action](https://github.com/taiki-e/install-action) can only be used when pre built binaries are available.
- [baptiste0928/cargo-install](https://github.com/baptiste0928/cargo-install) will compile the binary and cache it.

## Release macOS signing

The Release workflow signs and notarizes `x86_64-apple-darwin` and `aarch64-apple-darwin` binaries before uploading them for npm publishing.
Dry-run releases still sign and notarize macOS artifacts so the protected release path is exercised before publishing.

GitHub secrets:

- `APPLE_CERT_DATA`: base64-encoded Developer ID Application `.p12` certificate.
- `APPLE_CERT_PASSWORD`: password for the `.p12` certificate.
- `APPLE_API_KEY`: base64-encoded App Store Connect API key JSON for notarization.

The workflow signs with `rcodesign` from `apple-codesign` 0.29.0 using the binary identifier `com.vercel.turbo` and submits notarization with `rcodesign notary-submit --wait`.

## Versioned docs aliases

The Release workflow's `alias-versioned-docs` job requests a GitHub Actions OIDC token with audience `https://github.com/vercel` and exchanges it for a short-lived Vercel access token. It does not use the `TURBO_TOKEN` secret.

The Vercel CLI policy `Turborepo versioned docs aliases` (`pol_e1dcc1e2-c117-4b83-9dd0-2b812c7aaab2`) requires the `vercel/turborepo` repository, repository ID `413918947`, `refs/heads/main`, and workflow reference `vercel/turborepo/.github/workflows/turborepo-release.yml@refs/heads/main`. It grants `read:deployment`, `read-write:project`, and `read-write:domain`, with project operations restricted to `turbo-site`. The domain permission also grants team-level domain operations; the project boundary does not restrict those operations to `turborepo.dev`.

The job lists READY deployments for the release's base SHA and assigns the versioned hostname through the Vercel REST API. This avoids the user-only authentication checks in the previously pinned `vercel@53.1.0`. The domain must already belong to the Vercel team and have an active certificate covering the hostname.

Keep the job's team, project, and policy IDs aligned with the Vercel policy. If the repository, release workflow path, or release branch changes, update the policy's claims before running the release workflow. Dry runs skip this job, and a feature-branch run cannot match its `main`-only policy.
