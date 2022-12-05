import type { NextRequest } from "next/server";

const REGISTRY = "https://registry.npmjs.org";
const SUPPORTED_PACKAGES = ["turbo"];
const SUPPORTED_METHODS = ["POST"];
const [DEFAULT_PACKAGE] = SUPPORTED_PACKAGES;

const ERROR_TYPES = {
  FourOhFour: "FourOhFour",
  FourHundred: "FourHundred",
  FiveHundred: "FiveHundred",
};

const ERRORS = {
  [ERROR_TYPES.FourOhFour]: {
    status: 404,
    error: "Not Found",
  },
  [ERROR_TYPES.FourHundred]: {
    status: 400,
    error: "Unexpected input",
  },
  [ERROR_TYPES.FiveHundred]: {
    status: 500,
    error: "Internal Server Error",
  },
};

async function fetchVersion({ name }: { name: string }) {
  const result = await fetch(`${REGISTRY}/${name}/latest`);
  const json = await result.json();
  return json.version;
}

function errorResponse(type: keyof typeof ERRORS) {
  const { status, error } = ERRORS[type];
  return new Response(
    JSON.stringify({
      error,
    }),
    {
      status,
    }
  );
}

export default async function handler(req: NextRequest) {
  if (!SUPPORTED_METHODS.includes(req.method)) {
    return errorResponse(ERROR_TYPES.FourOhFour);
  }

  try {
    const body = await req.json();
    const { name = DEFAULT_PACKAGE } = body;
    if (!SUPPORTED_PACKAGES.includes(name)) {
      return errorResponse(ERROR_TYPES.FourHundred);
    }

    const version = await fetchVersion({ name });
    return new Response(
      JSON.stringify({
        version,
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
    return errorResponse(ERROR_TYPES.FiveHundred);
  }
}

export const config = {
  runtime: "experimental-edge",
};
