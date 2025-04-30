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
// Self-hosted: https://turborepo.com/api/remote-cache-spec

import { writeFileSync } from "node:fs";
import { generateFiles } from "fumadocs-openapi";

const out = "./content/openapi";

interface OpenAPISpec {
  paths?: Record<
    string,
    Record<
      string,
      {
        responses?: Record<
          string,
          {
            description?: string;
            headers?: Record<
              string,
              {
                schema: {
                  type: string;
                };
                description: string;
              }
            >;
          }
        >;
      }
    >
  >;
  servers?: Array<{
    url: string;
    description?: string;
  }>;
}

// Define a more specific type for the OpenAPI value structure
type OpenAPIValue =
  | string
  | number
  | boolean
  | null
  | { [key: string]: OpenAPIValue }
  | Array<OpenAPIValue>;

/* The Vercel Remote Cache spec has examples that show Vercel values.
 * Removing them makes the self-hosted spec easier to use. */
const removeExamples = (obj: OpenAPIValue): OpenAPIValue => {
  if (!obj || typeof obj !== "object") return obj;

  if (Array.isArray(obj)) {
    return obj.map((item) => removeExamples(item));
  }

  const result: Record<string, OpenAPIValue> = {};
  for (const [key, value] of Object.entries(
    obj as Record<string, OpenAPIValue>
  )) {
    if (key !== "example") {
      result[key] = removeExamples(value);
    }
  }

  return result;
};

/* The Vercel Remote Cache spec has responses related to billing.
 * Self-hosted users don't need these. */
function removeBillingRelated403Responses(spec: OpenAPISpec): OpenAPISpec {
  // Define billing-related phrases to filter out
  const billingPhrases = [
    "The customer has reached their spend cap limit and has been paused",
    "The Remote Caching usage limit has been reached for this account",
    "Remote Caching has been disabled for this team or user",
  ];

  // Process all paths
  for (const path in spec.paths) {
    const pathObj = spec.paths[path];

    // Process all methods in each path
    for (const method in pathObj) {
      const methodObj = pathObj[method];

      // Check if the method has responses
      if (methodObj.responses?.["403"]) {
        const description = methodObj.responses["403"].description;

        // Split the description by newlines
        const descriptionLines = description?.split("\n") ?? [];

        // Filter out billing-related lines
        const filteredLines = descriptionLines.filter((line) => {
          return !billingPhrases.some((phrase) => line.includes(phrase));
        });

        // If there are remaining lines, join them back together
        if (filteredLines.length > 0) {
          methodObj.responses["403"].description = filteredLines.join("\n");
        } else {
          // If all lines were billing-related, set a generic permission message
          methodObj.responses["403"].description =
            "You do not have permission to access this resource.";
        }
      }
    }
  }

  return spec;
}

/* Add x-artifact-tag header to artifact download endpoint response */
function addArtifactTagHeader(spec: OpenAPISpec): OpenAPISpec {
  // Target only the specific /v8/artifacts/{hash} endpoint
  const artifactEndpoint = "/v8/artifacts/{hash}";

  if (spec.paths?.[artifactEndpoint]) {
    // Get the GET method for this endpoint
    const getMethod = spec.paths[artifactEndpoint].get;

    if (getMethod.responses?.["200"]) {
      const response = getMethod.responses["200"];

      // Add headers to the response if they don't exist
      if (!response.headers) {
        response.headers = {};
      }

      // Add the x-artifact-tag header
      response.headers["x-artifact-tag"] = {
        schema: {
          type: "string",
        },
        description: "The signature of the artifact found",
      };
    }
  }

  return spec;
}

const updateServerDescription = (spec: OpenAPISpec): OpenAPISpec => {
  if (spec.servers && spec.servers.length > 0) {
    const serverIndex = spec.servers.findIndex(
      (server) => server.url === "https://api.vercel.com"
    );

    if (serverIndex !== -1) {
      spec.servers[serverIndex].description =
        "Vercel Remote Cache implementation for reference.";
    }
    return spec;
  }
  return spec;
};

const thing = await fetch("https://turborepo.com/api/remote-cache-spec")
  .then((res) => res.json())
  .then((json: unknown) => removeExamples(json as OpenAPIValue) as OpenAPISpec)
  .then((json) => removeBillingRelated403Responses(json))
  .then((json) => addArtifactTagHeader(json))
  .then((json) => updateServerDescription(json));

writeFileSync("./.openapi.json", JSON.stringify(thing, null, 2));

void generateFiles({
  input: ["./.openapi.json"],
  addGeneratedComment: true,
  output: out,
  groupBy: "tag",
});
