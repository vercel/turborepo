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
