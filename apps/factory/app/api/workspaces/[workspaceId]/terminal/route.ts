import { Sandbox } from "@vercel/sandbox";

import { createTerminalSession } from "../../../../../agent/lib/sandbox-terminal";
import { getWorkspace } from "../../../../../agent/lib/workspace-store";
import {
  isWorkspaceMutationRequest,
  WORKSPACE_TERMINAL_ACTION
} from "../../../../../agent/lib/workspace";

export async function POST(
  request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  if (
    !isWorkspaceMutationRequest(request, WORKSPACE_TERMINAL_ACTION, {
      requireJson: false
    })
  )
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });
  try {
    const sandbox = await Sandbox.get({ name: workspace.sandbox.name });
    const cwd = await repositoryDirectory(sandbox);
    return Response.json(
      {
        ...(await createTerminalSession(
          workspace.sandbox.name,
          async () => sandbox
        )),
        cwd
      },
      { headers: { "cache-control": "no-store" } }
    );
  } catch (error) {
    console.error("Could not open workspace terminal.", error);
    return Response.json(
      { error: "Could not open the workspace terminal." },
      { status: 502 }
    );
  }
}

async function repositoryDirectory(sandbox: Sandbox): Promise<string> {
  for (const cwd of ["turborepo", "."]) {
    const command = await sandbox.runCommand({
      args: ["rev-parse", "--show-toplevel"],
      cmd: "git",
      cwd,
      timeoutMs: 30_000
    });
    if (command.exitCode === 0) return cwd;
  }
  throw new Error("Workspace repository is unavailable.");
}
