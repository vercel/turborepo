import { readFactoryImageView } from "../../../agent/lib/factory-image-registry";
import { triggerFactoryImageBuild } from "../../../agent/lib/factory-image-trigger";
import { FACTORY_IMAGE_REBUILD_ACTION } from "../../../agent/lib/factory-image-types";
import { fetchMainCommit } from "../../../agent/lib/github";

/** Current factory image plus recent builds, for the operator dashboard. */
export async function GET(): Promise<Response> {
  return Response.json(await readFactoryImageView(), {
    headers: { "cache-control": "no-store" }
  });
}

/**
 * Operator-triggered rebuild of the image at the current `main` head.
 * Same-origin only, and guarded by the operator action header the
 * dashboard sends; the page itself sits behind Deployment Protection.
 */
export async function POST(request: Request): Promise<Response> {
  if (
    request.headers.get("origin") !== new URL(request.url).origin ||
    request.headers.get("x-operator-action") !== FACTORY_IMAGE_REBUILD_ACTION
  ) {
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  }

  try {
    const commit = await fetchMainCommit();
    const result = await triggerFactoryImageBuild({
      commit,
      ref: "refs/heads/main",
      trigger: "operator"
    });
    return Response.json(result, {
      headers: { "cache-control": "no-store" },
      status: result.state === "claimed" ? 202 : 200
    });
  } catch (error) {
    console.error("Could not start a factory image build.", error);
    return Response.json(
      { error: "Could not start a factory image build." },
      { status: 503 }
    );
  }
}
