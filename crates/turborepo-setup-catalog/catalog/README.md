# Bundled tool catalog

`turbo setup` uses these files to turn a tool name and version in `setup.additionalTools` into the downloads it records in `turbo.lock`. They are read only when `turbo setup` resolves a new or changed declaration. Installing from `turbo.lock` never reads them.

| File                    | Contents                                                                                                                   |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `aqua-registry.yaml.gz` | The merged recipe file of the [aqua registry](https://github.com/aquaproj/aqua-registry) at a release tag, gzip-compressed |
| `names.tsv`             | Short tool names mapped to aqua package IDs, taken from the [mise registry](https://github.com/jdx/mise/tree/main/registry) |
| `update.mjs`            | Regenerates both files                                                                                                     |

The header of `names.tsv` records the aqua registry tag and the mise commit that both files came from.

## Updating

```bash
node crates/turborepo-setup-catalog/catalog/update.mjs
cargo test -p turborepo-setup-catalog
```

Pass an aqua registry tag and a mise commit to pin either source. To map a name that mise doesn't list, or to map one differently, add it to `OVERRIDES` in `update.mjs`.

A repository's `turbo.lock` records the package ID and the exact downloads that each tool resolved to, so updating these files never changes what an existing lock installs.

## Licenses

The aqua registry and mise are MIT licensed. Their notices are in `LICENSE-aqua-registry` and `LICENSE-mise`.
