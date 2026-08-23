import { randomUUID } from "node:crypto";

import {
  createWorkspace,
  isWorkspaceStoreConfigured,
  listWorkspaces
} from "../../../agent/lib/workspace-store";
import { queueWorkspaceTurn } from "../../../agent/lib/workspace-turn";
import {
  isWorkspaceMutationRequest,
  parseCreateWorkspaceInput,
  toWorkspaceSummary,
  toWorkspaceView,
  WORKSPACE_CREATE_ACTION,
  workspaceSandboxName,
  type WorkspaceRecord
} from "../../../agent/lib/workspace";

export async function GET(): Promise<Response> {
  if (!isWorkspaceStoreConfigured()) return unconfigured();
  const workspaces = await listWorkspaces();
  return Response.json(
    { workspaces: workspaces.map(toWorkspaceSummary) },
    { headers: { "cache-control": "no-store" } }
  );
}

export async function POST(request: Request): Promise<Response> {
  if (!isWorkspaceMutationRequest(request, WORKSPACE_CREATE_ACTION))
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  if (!isWorkspaceStoreConfigured()) return unconfigured();

  const input = parseCreateWorkspaceInput(
    await request.json().catch(() => null)
  );
  if (!input)
    return Response.json(
      { error: "A title or prompt is required." },
      { status: 400 }
    );

  const id = `ws_${randomUUID().replaceAll("-", "")}`;
  const now = new Date().toISOString();
  const workspace: WorkspaceRecord = {
    agent: "fx",
    createdAt: now,
    id,
    messages: [],
    sandbox: {
      name: workspaceSandboxName(id),
      provider: "vercel",
      status: "pending"
    },
    status: "idle",
    title: input.title,
    updatedAt: now,
    version: 1
  };
  await createWorkspace(workspace);
  const view = input.prompt
    ? await queueWorkspaceTurn(workspace.id, input.prompt)
    : toWorkspaceView(workspace);
  return Response.json(view, {
    status: input.prompt ? 202 : 201,
    headers: { "cache-control": "no-store" }
  });
}

function unconfigured(): Response {
  return Response.json(
    { error: "Workspace storage is not configured." },
    { status: 503, headers: { "cache-control": "no-store" } }
  );
}
