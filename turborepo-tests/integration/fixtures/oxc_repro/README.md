# oxc symlink bug reproduction

The bug occurs when a symlink is nested inside a directory that is also symlinked.
This occurs with pnpm since it recreates a conventional node_modules structure using
a content addressed store and symlinks.

Here's the setup:

- `apps/web/nm/@repo/typescript-config` is a symlink pointing to `tooling/typescript-config` (imagine `typescript-config` is a workspace package and symlinked into `apps/web`'s node modules)
- `tooling/typescript-config/index.js` is a _relative_ symlink pointing to `../../nm/index.js`
- Therefore, `apps/web/nm/@repo/typescript-config/index.js` is resolved as:

```
apps/web/nm/@repo/typescript-config/index.js
-> tooling/typescript-config/index.js
-> tooling/typescript-config/../../nm/index.js
-> nm/index.js
```

However, when oxc resolves this, it does not do the first resolution, so we get:

```
apps/web/nm/@repo/typescript-config/index.js
-> apps/web/nm/@repo/typescript-config/../../nm/index.js
-> apps/web/nm/nm/index.js
```

You can validate this by running `node main.mjs`, which attempts to resolve
both `apps/web/nm/@repo/typescript-config/index.js` and `apps/web/nm/@repo/index.js`.
The first fails while the second succeeds.
