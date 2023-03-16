# nextpack

Nextpack is the development sub-repo, allowing us to develop Next.js
features and Turbopack together. This sub-repo will stitch together the
rust crates in Next.js and the crates in Turbo, so they appear as 1
logical Cargo workspace.

There are two steps:

```bash
# This will clone next.js into ./next.js
# if you wanted, you can link your own already-checked-out next.js repo.
# This will automatically run the next step.
$ node ./scripts/init.cjs

# This reads the Cargo.toml from Next.js and Turbo, generating a new
# Cargo.toml workspace in this directory.
$ cargo run --bin sync-workspace
```

## FAQs

### Failed to load manifest for workspace member …

You can't have an optional workspace member (or if you can, I can't
figure it out). So the `./next.js/…` crate members are gonna error out
if you try to run a Turbo crate without first cloning next.js.

To fix, run `node ./scripts/init.js` again. It'll resync your Cargo
workspace.

### Why is `init` written in JS?

See the above. If we wrote it in Rust, any failure in the repo setup
would prevent it from running.

### Why is there a turbo-crates symlink dir?

Cargo cannot contain nested workspaces, and if we used
`../crates/turbopack` as a workspace member, it would error out. By
using a symlink, we avoid the problem.

### How do I run `next dev --turbo` with this?

I'm still working on it.
