import { randomUUID } from "node:crypto";

import { defineChannel, GET, POST } from "eve/channels";

import { DAILY_EXAMPLE_MAINTENANCE_PROMPT } from "../lib/daily-example-maintenance.js";
import { DAILY_PERFORMANCE_IMPROVEMENT_PROMPT } from "../lib/daily-performance-improvement.js";
import {
  createOperatorWorkspaceRecord,
  isOperatorRunRequest,
  MAINTENANCE_RUN_ACTION,
  type OperatorRunAction,
  PERFORMANCE_RUN_ACTION
} from "../lib/operator-runs.js";
import { selectPerformanceModels } from "../lib/performance-models.js";
import { sessionDate } from "../lib/repo.js";
import { deliverSlackMessage } from "../lib/slack.js";
import { createWorkspace, mutateWorkspace } from "../lib/workspace-store.js";
import type { WorkspaceRecord } from "../lib/workspace.js";
import workspaceChannel from "./workspace.js";

const APP_AUTH = {
  attributes: {},
  authenticator: "app",
  principalId: "eve:app",
  principalType: "runtime"
} as const;

function operatorAction(request: Request): OperatorRunAction | null {
  const action = request.headers.get("x-operator-action");
  return (action === MAINTENANCE_RUN_ACTION ||
    action === PERFORMANCE_RUN_ACTION) &&
    isOperatorRunRequest(request, action)
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
  workspaceId: string,
  models?: { authorModel: string; reviewerModel: string }
): Response {
  return Response.json(
    { cursor: 0, models, sessionId, state: "running", workspaceId },
    { status: 202, headers: { "cache-control": "no-store" } }
  );
}

export default defineChannel({
  routes: [
    POST("/eve/v1/operator/runs", async (request, { to }) => {
      const action = operatorAction(request);
      if (!action) {
        return operatorError("Invalid operator request.", 403);
      }

      if (action === PERFORMANCE_RUN_ACTION) {
        const title = "Daily performance improvement";
        const workspace = await createOperatorWorkspace(
          title,
          DAILY_PERFORMANCE_IMPROVEMENT_PROMPT
        );
        const session = await to(workspaceChannel, {
          mode: "task",
          title,
          workspaceId: workspace.id
        })
          .send(DAILY_PERFORMANCE_IMPROVEMENT_PROMPT, { auth: APP_AUTH })
          .catch(async (error: unknown) => {
            await failOperatorWorkspace(
              workspace.id,
              "Could not start the scheduled performance session."
            );
            throw error;
          });
        await attachWorkspaceSession(workspace.id, session.id);
        const { authorModel, reviewerModel } = selectPerformanceModels(
          sessionDate(session.id)
        );

        return startedRun(session.id, workspace.id, {
          authorModel,
          reviewerModel
        });
      }

      const example = await requestedExample(request);
      if (!example) {
        return operatorError("A valid example is required.", 400);
      }

      const title = `Daily example maintenance · ${example}`;
      const workspace = await createOperatorWorkspace(
        title,
        DAILY_EXAMPLE_MAINTENANCE_PROMPT
      );
      const session = await to(workspaceChannel, {
        mode: "task",
        title,
        workspaceId: workspace.id
      })
        .send(DAILY_EXAMPLE_MAINTENANCE_PROMPT, {
          auth: {
            ...APP_AUTH,
            attributes: { maintenanceExample: example }
          }
        })
        .catch(async (error: unknown) => {
          await failOperatorWorkspace(
            workspace.id,
            "Could not start the scheduled maintenance session."
          );
          throw error;
        });
      await attachWorkspaceSession(workspace.id, session.id);

      return startedRun(session.id, workspace.id);
    }),
    POST("/eve/v1/operator/slack/test", async (request) => {
      if (!isOperatorRunRequest(request, "test-slack-delivery")) {
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
      async (request, { attachSession, params }) => {
        const cursorValue = new URL(request.url).searchParams.get("cursor");
        const startIndex = cursorValue === null ? 0 : Number(cursorValue);
        if (!Number.isInteger(startIndex) || startIndex < 0) {
          return Response.json(
            { error: "cursor must be a non-negative integer." },
            { status: 400, headers: { "cache-control": "no-store" } }
          );
        }

        const reader = (
          await attachSession(params.sessionId).getEventStream({ startIndex })
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

async function createOperatorWorkspace(
  title: string,
  prompt: string
): Promise<WorkspaceRecord> {
  return createWorkspace(
    createOperatorWorkspaceRecord({
      id: `ws_${randomUUID().replaceAll("-", "")}`,
      now: new Date().toISOString(),
      prompt,
      title,
      turnId: `turn_${randomUUID().replaceAll("-", "")}`
    })
  );
}

async function attachWorkspaceSession(
  workspaceId: string,
  sessionId: string
): Promise<void> {
  await mutateOperatorWorkspace(workspaceId, (workspace) => ({
    ...workspace,
    sessionId
  }));
}

async function failOperatorWorkspace(
  workspaceId: string,
  message: string
): Promise<void> {
  if (!workspaceId) return;
  await mutateOperatorWorkspace(workspaceId, (workspace) => ({
    ...workspace,
    activeTurnId: undefined,
    error: message.slice(0, 2000),
    sandbox: { ...workspace.sandbox, status: "error" },
    status: "error",
    updatedAt: new Date().toISOString()
  }));
}

async function mutateOperatorWorkspace(
  workspaceId: string,
  mutation: (workspace: WorkspaceRecord) => WorkspaceRecord
): Promise<void> {
  await mutateWorkspace(workspaceId, mutation).catch(() => undefined);
}
