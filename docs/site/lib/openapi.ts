import { createOpenAPI } from "fumadocs-openapi/server";
// @ts-expect-error - Using .ts extension for node --experimental-strip-types in generate script
import { fetchOpenAPISpec } from "./openapi-spec.ts";

export const openapi = createOpenAPI({
  // Use a function to provide the transformed spec dynamically
  input: async () => {
    const spec = await fetchOpenAPISpec();
    return {
      "remote-cache": spec
    };
  }
});
