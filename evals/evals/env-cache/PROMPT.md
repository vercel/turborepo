The `web` build embeds `STOREFRONT_API_ORIGIN` into its output. Building for preview and then for production can incorrectly reuse the preview result, and strict environment mode may silently fall back to the local URL.

Fix the monorepo build configuration so the variable is available to `web` and a different value invalidates `web`'s build cache. The independent `@repo/ui` library doesn't use it and shouldn't need to rebuild. Keep build outputs cached.
