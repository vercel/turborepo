# Mock official setup origins

`setup_run_journey.rs` routes the real binary's verified downloads through a
local CONNECT proxy using the standard `HTTPS_PROXY` and `SSL_CERT_FILE`
settings. The proxy terminates TLS with these committed certificates and serves
fixture bytes; it never forwards to a vendor. The private key is intentionally
public test data. Never trust this CA outside tests. The CA signing key was not
retained.

| File | Purpose |
| --- | --- |
| `ca.pem` | Self-signed P-256 CA (`CA:TRUE`, `keyCertSign,cRLSign`), valid for 100 years |
| `server.pem` | P-256 leaf for `DNS:nodejs.org` and `DNS:registry.npmjs.org`, `serverAuth` |
| `server-key.pem` | PKCS#8 key for `server.pem` |

Generated with OpenSSL 3 `genpkey` (P-256), `req -x509` for the CA and
`x509 -req -extfile` for the leaf. Inspect with:

```sh
openssl verify -CAfile ca.pem server.pem
openssl x509 -in server.pem -noout -dates -ext subjectAltName
```
