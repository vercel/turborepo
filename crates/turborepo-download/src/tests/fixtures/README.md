# Loopback TLS fixtures

These committed PEM files make tests deterministic; tests never regenerate them,
run OpenSSL, fetch certificates, or contact an external upstream. The private key
is intentionally public test data. Never use it for a real service or trust this
CA outside tests. The CA signing key is not retained.

| File | Purpose | Fixed validity (UTC) |
| --- | --- | --- |
| `ca.pem` | Self-signed P-256 CA, serial 1 | 2020-01-01 through 2120-01-01 |
| `server.pem` | P-256 server leaf, serial 2 | 2020-01-01 through 2120-01-01 |
| `expired.pem` | Same server key, serial 3 | 2000-01-01 through 2001-01-01 |
| `server-key.pem` | PKCS#8 P-256 test key | Not applicable |
| `invalid-der.pem` | Valid PEM containing a truncated DER SEQUENCE (`0x30`) | Not a certificate |

The invalid-DER fixture exercises reqwest's native-root build failure, unlike
plain junk that a PEM reader skips. Environment regressions use subprocesses.

Both leaves have SANs `DNS:localhost` and `IP:127.0.0.1`, serverAuth EKU,
CA:false, and digitalSignature key usage. The CA has CA:true and
keyCertSign/cRLSign key usage. Certificates were generated locally with OpenSSL
`req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256`, then `ca -selfsign`
for the CA and `ca` for leaves with explicit `-startdate` / `-enddate` values.
Expiry and hostname verification are tested without weakening the TLS verifier.

To inspect the fixture locally:

```sh
openssl verify -CAfile ca.pem server.pem
openssl x509 -in server.pem -noout -dates -serial -ext subjectAltName
```
