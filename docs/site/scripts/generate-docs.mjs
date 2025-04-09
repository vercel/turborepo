// The OpenAPI spec for a self-hosted implementation is generated
// from the Vercel Remote Cache implementation.
// The Vercel Remote Cache spec is more specific to the needs of Vercel
// while the self-hosted spec is more open for anyone to implement.
//
// While the two specifications are related enough to use the Vercel Remote Cache
// as the source of truth, the self-hosted Remote Cache has all
// of the same capabilities. Because of this,
// we do some light processing to make sure that the content
// in the self-hosted spec makes sense for self-hosted users.
//
// You can verify differences for the specs by comparing:
// Vercel Remote Cache: https://vercel.com/docs/rest-api/reference/endpoints/artifacts/record-an-artifacts-cache-usage-event
// Self-hosted: https://turbo.build/api/remote-cache-spec

import { writeFileSync } from "node:fs";
import { generateFiles } from "fumadocs-openapi";

const out = "./content/openapi";

/* The Vercel Remote Cache spec has examples that show Vercel values.
 * Removing them makes the self-hosted spec easier to use. */
const removeExamples = (obj) => {
  if (!obj || typeof obj !== "object") return obj;

  if (Array.isArray(obj)) {
    return obj.map((item) => removeExamples(item));
  }

  const result = {};
  for (const [key, value] of Object.entries(obj)) {
    if (key !== "example") {
      result[key] = removeExamples(value);
    }
  }

  return result;
};

const thing = await fetch("https://turbo.build/api/remote-cache-spec")
  .then((res) => res.json())
  .then((json) => removeExamples(json));

writeFileSync("./.openapi.json", JSON.stringify(thing, null, 2));

void generateFiles({
  input: ["./.openapi.json"],
  addGeneratedComment: true,
  output: out,
  groupBy: "tag",
});
