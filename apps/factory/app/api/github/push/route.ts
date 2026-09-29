import { vercelOidc } from "eve/channels/auth";

import { triggerFactoryImageBuild } from "../../../../agent/lib/factory-image-trigger";
import {
  isFactoryImageConnector,
  parseGitHubPush
} from "../../../../agent/lib/github-push";

const authenticateConnect = vercelOidc();

/**
 * GitHub push webhook. Every merge to `main` rebuilds the factory image
 * from here, so no GitHub Actions job is involved.
 *
 * Register `/api/github/push` as a Vercel Connect trigger destination for a
 * GitHub connector subscribed to `push`. Connect verifies GitHub's signature;
 * this route verifies the OIDC credential Connect adds while forwarding it.
 */
export async function POST(request: Request): Promise<Response> {
  const auth = await Promise.resolve(authenticateConnect(request)).catch(
    () => null
  );
  if (!isFactoryImageConnector(auth)) {
    return respond({ error: "Unauthorized webhook." }, 401);
  }

  const body = await request.text();
  const outcome = parseGitHubPush(request.headers.get("x-github-event"), body);
  if (outcome.kind === "ping") return respond({ pong: true }, 200);
  if (outcome.kind === "ignored") {
    return respond({ ignored: outcome.reason }, 200);
  }

  try {
    const result = await triggerFactoryImageBuild({
      commit: outcome.push.commit,
      ref: outcome.push.ref,
      trigger: "webhook"
    });
    return respond(result, result.state === "claimed" ? 202 : 200);
  } catch (error) {
    console.error("Could not start a factory image build.", error);
    return respond({ error: "Could not start a factory image build." }, 503);
  }
}

function respond(body: unknown, status: number): Response {
  return Response.json(body, {
    headers: { "cache-control": "no-store" },
    status
  });
}
