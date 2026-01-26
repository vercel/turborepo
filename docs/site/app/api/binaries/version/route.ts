import type { NextRequest } from "next/server";

type NonEmptyArray<T> = Array<T> & { 0: T };

const REGISTRY = "https://registry.npmjs.org";
const DEFAULT_TAG = "latest";
const SUPPORTED_PACKAGES: NonEmptyArray<string> = ["turbo"];
const SUPPORTED_METHODS = ["GET"];
const [DEFAULT_NAME] = SUPPORTED_PACKAGES;

/**
 * Sanitize a value before including it in an error message.
 * Only allow a limited set of safe characters; otherwise, replace with a placeholder.
 */
function sanitizeForErrorMessage(value: string): string {
  // Allow word characters, dots and dashes; everything else is treated as invalid.
  return /^[\w.-]+$/.test(value) ? value : "<invalid>";
}

// There are other properties returned
// but this is the one we care about.
interface FetchDistTags {
  "dist-tags": {
    latest: string;
    next: string;
    canary: string;
  };
}

export async function fetchDistTags({
  name
}: {
  name: string;
}): Promise<FetchDistTags["dist-tags"]> {
  const result = await fetch(`${REGISTRY}/${name}`);
  const json = (await result.json()) as FetchDistTags;
  return json["dist-tags"];
}

function errorResponse({
  status,
  message
}: {
  status: 400 | 404 | 500;
  message: string;
}): Response {
  return new Response(
    JSON.stringify({
      error: message
    }),
    {
      status
    }
  );
}

/*
This API is called via the turbo rust binary to check for version updates.

Response Schema (200):
{
    "type": "object",
    "properties": {
        "name": {
            "type": "string",
        },
        "version": {
            "type": "string",
        },
        "tag": {
            "type": "string",
        }
    }
}

Errors (400 | 404 | 500):
{
    "type": "object",
    "properties": {
        "error": {
            "type": "string",
        }
    }
}

*/

export async function GET(req: NextRequest): Promise<Response> {
  if (!SUPPORTED_METHODS.includes(req.method)) {
    const safeMethod = sanitizeForErrorMessage(req.method);
    return errorResponse({
      status: 404,
      message: `unsupported method - ${safeMethod}`
    });
  }

  try {
    const { searchParams } = new URL(req.url);
    const name = searchParams.get("name") || DEFAULT_NAME;
    const safeName = sanitizeForErrorMessage(name);
    const safeTag = sanitizeForErrorMessage(String(tag));
    const tag = (searchParams.get("tag") ||
      DEFAULT_TAG) as keyof FetchDistTags["dist-tags"];

    if (!SUPPORTED_PACKAGES.includes(name)) {
      return errorResponse({
        status: 400,
        message: `unsupported package - ${safeName}`
      });
    }

    const versions = await fetchDistTags({ name });
    if (!versions[tag]) {
      return errorResponse({
        status: 404,
        message: `unsupported tag - ${safeTag}`
      });
    }

    return new Response(
      JSON.stringify({
        name,
        version: versions[tag],
        tag
      }),
      {
        status: 200,
        headers: {
          "content-type": "application/json",
          // cache for 15 minutes, and allow stale responses for 5 minutes
          "cache-control": "public, s-maxage=900, stale-while-revalidate=300"
        }
      }
    );
  } catch (e) {
    const error = e as Error;

    // eslint-disable-next-line no-console -- We're alright with this.
    console.error(error);
    return errorResponse({
      status: 500,
      message: sanitizeForErrorMessage(error.message)
    });
  }
}
