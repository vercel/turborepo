CI fetches the catalog service's current schema before building `web`. When the service returns to an earlier schema, the build sometimes ships a stale catalog from cache.

Make `npm run build` produce the catalog for the schema fetched during that run. For schema versions A, B, B, A, all four outputs must be correct. The repeated B build must reuse its cached result, even if `apps/web/dist` has been deleted.

Changes to `apps/web/src/render.cjs` must update the catalog and invalidate the web build. Schema changes and web source edits must leave the unrelated `@repo/ui` build cached after its first build. Both packages must still restore their output files from cache.

Keep the existing build and code-generation scripts, workspace layout, and dependencies. The service must still be fetched on every build. Do not disable build caching or change the build command.

For local reproduction, run `npm run catalog-service` in a separate terminal. It serves `tools/catalog-schema.json` and rereads that file on each request. Then run:

```sh
CATALOG_SCHEMA_URL=http://127.0.0.1:4319/schema npm run build
```

CI provides `CATALOG_SCHEMA_URL`. The service can change its response without changing any repository file or the URL.
