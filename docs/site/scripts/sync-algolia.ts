import * as fs from "node:fs";
import algosearch from "algoliasearch";
import env from "@next/env";
import { sync, type DocumentRecord } from "fumadocs-core/search/algolia";

// We assume you're working in development if this is not provided.
if (!process.env.NEXT_PUBLIC_ALGOLIA_INDEX) {
  env.loadEnvConfig(process.cwd());
}

// If you are targeting the development environment,
// you can get this key from the `turbo-site` project on Vercel
// if you are a part of the Vercel team.
if (!process.env.ALGOLIA_API_KEY) {
  throw new Error("No ALGOLIA_API_KEY provided.");
}

const ALGOLIA_INDEX_NAME = process.env.NEXT_PUBLIC_ALGOLIA_INDEX ?? "_docs_dev";

const content = fs.readFileSync(".next/server/app/static.json.body");

const indexes = (
  JSON.parse(content.toString()) as Array<DocumentRecord>
).filter(
  // These path don't have information that we think people want in search.
  (doc) => !["docs/community", "/docs"].includes(doc.url)
);

if (!process.env.ALGOLIA_APP_ID) {
  throw new Error("No ALGOLIA_APP_ID found.");
}

const algoliaClient = algosearch(
  process.env.ALGOLIA_APP_ID,
  process.env.ALGOLIA_API_KEY
);

// Ensure a non-production target environment cannot sync to a production-looking Algolia index
if (
  process.env.VERCEL_TARGET_ENV !== "production" &&
  process.env.NEXT_PUBLIC_ALGOLIA_INDEX?.includes("production")
) {
  throw new Error(
    "Writing to the production Algolia index when building for an environment that is not production is not permitted."
  );
}

const getDomain = () => {
  // Only "Production" environment
  if (process.env.VERCEL_TARGET_ENV === "production") {
    return `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`;
  }

  // All other environments on Vercel (Previews and Custom Environments)
  if (process.env.VERCEL_ENV) {
    return `https://${process.env.VERCEL_URL}`;
  }

  // For local development
  return `http://localhost:${process.env.PORT || 3000}`;
};

void sync(algoliaClient, {
  document: ALGOLIA_INDEX_NAME,
  documents: indexes.map((ind) => {
    return {
      ...ind,
      url: `${getDomain()}${ind.url}`,
      tag: ind.url.split("/")[1],
    };
  }),
})
  .then(() => {
    console.log(`Search index updated for ${ALGOLIA_INDEX_NAME}.`);
  })
  .catch((err: unknown) => {
    console.error(err);
    throw err instanceof Error ? err : new Error(String(err));
  });
