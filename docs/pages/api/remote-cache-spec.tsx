import type { OpenAPIV3 } from "openapi-types";

const VERCEL_OPEN_API = "https://openapi.vercel.sh/";
const API_VERSION = "8";
const API_PREFIX = `/v${API_VERSION}/artifacts`;
const INFO: OpenAPIV3.InfoObject = {
  title: "Turborepo Remote Cache API",
  description:
    "Turborepo is an intelligent build system optimized for JavaScript and TypeScript codebases.",
  version: `${API_VERSION}.0.0`,
};
const COMPONENTS: OpenAPIV3.ComponentsObject = {
  securitySchemes: {
    bearerToken: {
      type: "http",
      description: "Default authentication mechanism",
      scheme: "bearer",
    },
  },
};

async function fetchVercelOpenAPISchema(): Promise<OpenAPIV3.Document> {
  const result = await fetch(VERCEL_OPEN_API);
  const json = (await result.json()) as OpenAPIV3.Document;

  return json;
}

function formatOpenAPISchema(schema: OpenAPIV3.Document) {
  const paths: OpenAPIV3.PathsObject = {};
  for (const [path, methods] of Object.entries(schema.paths)) {
    if (path.startsWith(API_PREFIX)) {
      paths[path] = methods;
    }
  }

  // replace the paths, info and components
  schema.components = COMPONENTS;
  schema.info = INFO;
  schema.paths = paths;

  return schema;
}

function errorResponse({
  status,
  message,
}: {
  status: 400 | 404 | 500;
  message: string;
}) {
  return new Response(
    JSON.stringify({
      error: message,
    }),
    {
      status,
    }
  );
}
export default async function handler() {
  try {
    const vercelSchema = await fetchVercelOpenAPISchema();
    const remoteCacheSchema = formatOpenAPISchema(vercelSchema);

    return new Response(JSON.stringify(remoteCacheSchema), {
      status: 200,
      headers: {
        "content-type": "application/json",
        // cache for one day, and allow stale responses for one hour
        "cache-control": `public, s-maxage=${
          60 * 60 * 24
        }, stale-while-revalidate=${60 * 60}`,
      },
    });
  } catch (e) {
    const error = e as Error;

    // eslint-disable-next-line no-console -- We're alright with this.
    console.error(error);
    return errorResponse({ status: 500, message: error.message });
  }
}

export const config = {
  runtime: "edge",
};
