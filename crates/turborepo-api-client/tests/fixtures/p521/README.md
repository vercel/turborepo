# P-521 certificate fixtures

These reproduce the certificate chain from
[issue #13035](https://github.com/vercel/turborepo/issues/13035): a P-521
(`secp521r1`) ECDSA certificate authority signing a P-384 leaf with
`ecdsa-with-SHA256`. rustls' default `ring` crypto provider cannot verify P-521
signatures, so this chain is rejected with
`UnsupportedSignatureAlgorithmForPublicKeyContext` until the augmented provider
in `src/tls.rs` is installed.

- `ca.crt` — self-signed P-521 ECDSA CA.
- `server.crt` — `localhost` P-384 leaf signed by the CA using SHA-256.
- `server.key` — the leaf's private key (PKCS#8), for spinning up a test server.

Regenerate with:

```sh
# P-521 ECDSA CA
openssl ecparam -name secp521r1 -genkey -noout -out ca.key
openssl req -x509 -new -key ca.key -sha256 -days 3650 -subj "/CN=Test P521 CA" -out ca.crt

# P-384 leaf signed by the CA with SHA-256
openssl ecparam -name secp384r1 -genkey -noout -out leaf.key
openssl req -new -key leaf.key -subj "/CN=localhost" -out leaf.csr
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > ext.cnf
openssl x509 -req -in leaf.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 3650 -sha256 -extfile ext.cnf -out server.crt
openssl pkcs8 -topk8 -nocrypt -in leaf.key -out server.key
```
