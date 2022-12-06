import type { NextRequest } from "next/server";

const REGISTRY = "https://registry.npmjs.org";
const DEFAULT_TAG = "latest";
const SUPPORTED_PACKAGES = ["turbo"];
const SUPPORTED_METHODS = ["POST"];
const [DEFAULT_PACKAGE] = SUPPORTED_PACKAGES;

async function fetchDistTags({ name }: { name: string }) {
  const result = await fetch(`${REGISTRY}/${name}`);
  const json = await result.json();
  return json["dist-tags"];
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

/*
This API is called via the turbo rust binary to check for version updates.

We use a POST here instead of a GET because we may want to eventually have more
granular control over when the notifications for new releases are presented to
users. A POST allows a simpler way to send arbitrary data to the API in a
more backwards compatible way, which we can then use to decide when to
display the notification.

Request Schema:
{
    "type": "object",
    "properties": {
        "name": {
            "type": "string",
            "default": "turbo"
        },
        "tag": {
            "type": "string",
            "default": "latest"
        }
    }
}

Response Schema (200):
{
    "type": "object",
    "properties": {
        "name": {
            "type": "string",
        },
        "version": {
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
export default async function handler(req: NextRequest) {
  if (!SUPPORTED_METHODS.includes(req.method)) {
    return errorResponse({
      status: 404,
      message: `unsupported method - ${req.method}`,
    });
  }

  try {
    const body = await req.json();
    const { name = DEFAULT_PACKAGE, tag = DEFAULT_TAG } = body;
    if (!SUPPORTED_PACKAGES.includes(name)) {
      return errorResponse({
        status: 400,
        message: `unsupported package - ${name}`,
      });
    }

    const versions = await fetchDistTags({ name });
    if (!versions || !versions[tag]) {
      return errorResponse({
        status: 404,
        message: `unsupported tag - ${tag}`,
      });
    }

    return new Response(
      JSON.stringify({
        version: versions[tag],
        tag,
        name,
      }),
      {
        status: 200,
        headers: {
          "content-type": "application/json",
        },
      }
    );
  } catch (e) {
    console.error(e);
    return errorResponse({ status: 500, message: e.message });
  }
}

export const config = {
  runtime: "experimental-edge",
};
