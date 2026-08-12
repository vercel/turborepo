import { randomUUID } from "node:crypto";

import { defineChannel, GET, POST } from "eve/channels";

import { DAILY_EXAMPLE_MAINTENANCE_PROMPT } from "../lib/daily-example-maintenance.js";
import { DAILY_PERFORMANCE_IMPROVEMENT_PROMPT } from "../lib/daily-performance-improvement.js";
import {
  MAINTENANCE_RUN_ACTION,
  type OperatorRunAction,
  PERFORMANCE_RUN_ACTION
} from "../lib/operator-runs.js";
import { selectPerformanceModels } from "../lib/performance-models.js";
import { sessionDate } from "../lib/repo.js";

const APP_AUTH = {
  attributes: {},
  authenticator: "app",
  principalId: "eve:app",
  principalType: "runtime"
} as const;

function operatorAction(request: Request): OperatorRunAction | null {
  if (
    request.headers.get("origin") !== new URL(request.url).origin ||
    !request.headers.get("content-type")?.startsWith("application/json")
  ) {
    return null;
  }

  const action = request.headers.get("x-operator-action");
  return action === MAINTENANCE_RUN_ACTION || action === PERFORMANCE_RUN_ACTION
    ? action
    : null;
}

async function requestedExample(request: Request): Promise<string | null> {
  const body: unknown = await request.json().catch(() => null);
  return typeof body === "object" &&
    body !== null &&
    "example" in body &&
    typeof body.example === "string" &&
    /^[A-Za-z0-9._-]+$/.test(body.example)
    ? body.example
    : null;
}

function operatorError(error: string, status: number): Response {
  return Response.json(
    { ok: false, error },
    { status, headers: { "cache-control": "no-store" } }
  );
}

function startedRun(
  sessionId: string,
  models?: { authorModel: string; reviewerModel: string }
): Response {
  return Response.json(
    { cursor: 0, models, sessionId, state: "running" },
    { status: 202, headers: { "cache-control": "no-store" } }
  );
}

export default defineChannel({
  routes: [
    POST("/eve/v1/operator/runs", async (request, { send }) => {
      const action = operatorAction(request);
      if (!action) {
        return operatorError("Invalid operator request.", 403);
      }

      if (action === PERFORMANCE_RUN_ACTION) {
        const session = await send(DAILY_PERFORMANCE_IMPROVEMENT_PROMPT, {
          auth: APP_AUTH,
          continuationToken: randomUUID(),
          mode: "task",
          title: "Daily performance improvement"
        });
        const { authorModel, reviewerModel } = selectPerformanceModels(
          sessionDate(session.id)
        );

        return startedRun(session.id, { authorModel, reviewerModel });
      }

      const example = await requestedExample(request);
      if (!example) {
        return operatorError("A valid example is required.", 400);
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

      return startedRun(session.id);
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
