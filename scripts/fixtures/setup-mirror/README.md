# Loopback setup mirror certificates

`scripts/setup-fixture-mirror.mjs` serves the release smokes' fixture Node.js,
pnpm, Rust, uv, Python, Go, and additional-tool artifacts over HTTPS at
`127.0.0.1`. The smokes point `TURBO_SETUP_NPM_REGISTRY` and the
`TURBO_SETUP_*_MIRROR` variables at it and trust `ca.pem` through
`SSL_CERT_FILE` for turbo only. The private key is
intentionally public test data. Never trust this CA outside the smokes. The CA
signing key was not retained.

| File             | Purpose                                                          |
| ---------------- | ---------------------------------------------------------------- |
| `ca.pem`         | Self-signed P-256 CA (`CA:TRUE`, `keyCertSign,cRLSign`), 100 years |
| `server.pem`     | P-256 leaf for `IP:127.0.0.1`, `serverAuth`                      |
| `server-key.pem` | PKCS#8 key for `server.pem`                                      |
| `standin.cs`     | Windows stand-in tools, compiled by the mirror with `csc.exe`    |

Generated with OpenSSL 3 `genpkey` (P-256), `req -x509` for the CA and
`x509 -req -extfile` for the leaf. Inspect with:

```sh
openssl verify -CAfile ca.pem server.pem
openssl x509 -in server.pem -noout -dates -ext subjectAltName
```
