import { randomUUID } from "node:crypto";

import { defineChannel, GET, POST } from "eve/channels";

import { DAILY_EXAMPLE_MAINTENANCE_PROMPT } from "../lib/daily-example-maintenance.js";
import { deliverSlackMessage } from "../lib/slack.js";

const APP_AUTH = {
  attributes: {},
  authenticator: "app",
  principalId: "eve:app",
  principalType: "runtime"
} as const;

function isOperatorRequest(request: Request, action: string): boolean {
  // Vercel Deployment Protection authenticates operators; this validates CSRF intent.
  return (
    request.headers.get("origin") === new URL(request.url).origin &&
    request.headers.get("x-operator-action") === action &&
    request.headers.get("content-type")?.split(";", 1)[0] === "application/json"
  );
}

export default defineChannel({
  routes: [
    POST("/eve/v1/operator/runs", async (request, { send }) => {
      if (!isOperatorRequest(request, "run-daily-maintenance")) {
        return Response.json(
          { ok: false, error: "Invalid operator request." },
          {
            status: 403,
            headers: { "cache-control": "no-store" }
          }
        );
      }

      const body: unknown = await request.json().catch(() => null);
      const example =
        typeof body === "object" &&
        body !== null &&
        "example" in body &&
        typeof body.example === "string" &&
        /^[A-Za-z0-9._-]+$/.test(body.example)
          ? body.example
          : null;
      if (!example) {
        return Response.json(
          { ok: false, error: "A valid example is required." },
          { status: 400, headers: { "cache-control": "no-store" } }
        );
      }

      const session = await send(DAILY_EXAMPLE_MAINTENANCE_PROMPT, {
        auth: {
          ...APP_AUTH,
          attributes: { maintenanceExample: example }
        },
        continuationToken: randomUUID(),
        mode: "task",
        title: "Daily example maintenance"
      });

      return Response.json(
        { cursor: 0, sessionId: session.id, state: "running" },
        { status: 202, headers: { "cache-control": "no-store" } }
      );
    }),
    POST("/eve/v1/operator/slack/test", async (request) => {
      if (!isOperatorRequest(request, "test-slack-delivery")) {
        return Response.json(
          { ok: false, error: "Invalid operator request." },
          {
            status: 403,
            headers: { "cache-control": "no-store" }
          }
        );
      }

      const result = await deliverSlackMessage(
        `Turborepo Eve operator test at ${new Date().toISOString()}.`,
        { event: "operator_slack_test" }
      );
      return Response.json(result, {
        status: result.ok ? 200 : 502,
        headers: { "cache-control": "no-store" }
      });
    }),
    GET(
      "/eve/v1/operator/runs/:sessionId/status",
      async (request, { getSession, params }) => {
        const cursorValue = new URL(request.url).searchParams.get("cursor");
        const startIndex = cursorValue === null ? 0 : Number(cursorValue);
        if (!Number.isInteger(startIndex) || startIndex < 0) {
          return Response.json(
            { error: "cursor must be a non-negative integer." },
            { status: 400, headers: { "cache-control": "no-store" } }
          );
        }

        const reader = (
          await getSession(params.sessionId).getEventStream({ startIndex })
        ).getReader();
        let cursor = startIndex;
        let state: "running" | "done" | "error" = "running";
        const timeout = setTimeout(() => {
          void reader.cancel();
        }, 250);

        try {
          while (true) {
            const { done, value: event } = await reader.read();
            if (done) break;
            cursor += 1;
            if (event.type === "session.completed") {
              state = "done";
              break;
            }
            if (event.type === "session.failed") {
              state = "error";
              break;
            }
          }
        } finally {
          clearTimeout(timeout);
          await reader.cancel();
        }

        return Response.json(
          { cursor, sessionId: params.sessionId, state },
          { headers: { "cache-control": "no-store" } }
        );
      }
    )
  ]
});
