import { getVercelOidcToken } from "@vercel/oidc";

import { FACTORY_IMAGE_SPEC } from "../../../../../agent/lib/factory-image";
import {
  cancelFxAcpTurn,
  countFxSessions,
  prepareFxInteractiveLaunch
} from "../../../../../agent/lib/fx-interactive";
import { getFxWorkspaceSandbox } from "../../../../../agent/lib/fx-workspace";
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
  if (workspace.status === "running" && !workspace.sessionId)
    return Response.json(
      {
        code: "chat_initializing",
        error: "Factory is creating the first chat for this sandbox."
      },
      {
        status: 503,
        headers: { "cache-control": "no-store", "retry-after": "2" }
      }
    );
  try {
    const sandbox = await getFxWorkspaceSandbox(workspace.sandbox.name);
    if (workspace.status === "running") {
      await cancelFxAcpTurn(sandbox);
      return Response.json(
        {
          code: "chat_handoff",
          error: "Factory is handing the active chat to this terminal."
        },
        {
          status: 503,
          headers: { "cache-control": "no-store", "retry-after": "1" }
        }
      );
    }
    if (!workspace.sessionId) {
      const sessionCount = await countFxSessions(
        sandbox,
        FACTORY_IMAGE_SPEC.checkoutPath
      );
      return Response.json(
        sessionCount > 0
          ? {
              code: "untracked_chat",
              error:
                "This sandbox has an fx chat, but it was started outside Factory and is not linked to this workspace."
            }
          : {
              code: "chat_missing",
              error: "No fx chat has been created for this sandbox yet."
            },
        { status: 409, headers: { "cache-control": "no-store" } }
      );
    }
    const launch = await prepareFxInteractiveLaunch(
      sandbox,
      workspace.sessionId,
      getVercelOidcToken
    );
    return Response.json(
      {
        ...(await createTerminalSession(
          workspace.sandbox.name,
          async () => sandbox
        )),
        ...launch,
        cwd: FACTORY_IMAGE_SPEC.checkoutPath
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
