import { triggerFactoryImageBuild } from "../../../../agent/lib/factory-image-trigger";
import {
  githubWebhookSecret,
  parseGitHubPush,
  verifyGitHubSignature
} from "../../../../agent/lib/github-push";

/**
 * GitHub push webhook. Every merge to `main` rebuilds the factory image
 * from here, so no GitHub Actions job is involved.
 *
 * Point a repository webhook (or the GitHub App, subscribed to `push`) at
 * `/api/github/push` with `FACTORY_IMAGE_WEBHOOK_SECRET` as its secret.
 * Deployment Protection must not cover this route; the HMAC signature is
 * what authenticates the delivery.
 */
export async function POST(request: Request): Promise<Response> {
  const secret = githubWebhookSecret();
  if (secret === undefined) {
    return respond(
      { error: "The factory image webhook is not configured." },
      503
    );
  }

  const body = await request.text();
  if (
    !verifyGitHubSignature(
      body,
      request.headers.get("x-hub-signature-256"),
      secret
    )
  ) {
    return respond({ error: "Invalid signature." }, 401);
  }

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
