import { get, put } from "@vercel/blob";

import { getGitHubToken } from "./github.js";
import { listWorkspaces } from "./workspace-store.js";
import type { WorkspaceRecord } from "./workspace.js";

const PREFIX = "factory-workspace-status/v1/";
const REFRESH_INTERVAL_MS = 10 * 60 * 1000;

type PullRequestState = "open" | "closed" | "merged";

interface DisplayStatus {
  readonly activity?: string;
  readonly done?: boolean;
  readonly pullRequest?: {
    readonly checkedAt?: string;
    readonly number: number;
    readonly state?: PullRequestState;
    readonly url: string;
  };
}

export async function updateWorkspaceActivityForSession(
  sessionId: string,
  activity: string | undefined,
  startsTurn = false
): Promise<void> {
  const workspace = (await listWorkspaces()).find(
    (candidate) => candidate.sessionId === sessionId
  );
  if (!workspace) return;
  const current = await readStatus(workspace.id);
  await writeStatus(workspace.id, {
    ...current,
    ...(activity === undefined ? { activity: undefined } : { activity }),
    ...(startsTurn && current.pullRequest?.state === "merged"
      ? { done: false }
      : {})
  });
}

export async function recordWorkspacePullRequestForSession(
  sessionId: string,
  pullRequest: { readonly number: number; readonly url: string }
): Promise<void> {
  const workspace = (await listWorkspaces()).find(
    (candidate) => candidate.sessionId === sessionId
  );
  if (!workspace) return;
  const current = await readStatus(workspace.id);
  if (
    current.pullRequest?.number === pullRequest.number &&
    current.pullRequest.state === "merged"
  )
    return;
  await writeStatus(workspace.id, {
    ...current,
    done: false,
    pullRequest: {
      ...pullRequest,
      checkedAt: new Date().toISOString(),
      state: "open"
    }
  });
}

export async function workspaceDisplayStatuses(): Promise<
  Record<string, DisplayStatus>
> {
  const workspaces = await listWorkspaces();
  const entries = await Promise.all(
    workspaces.map(async (workspace) => {
      const current = await readStatus(workspace.id);
      const pullRequest = current.pullRequest ?? findPullRequest(workspace);
      const refreshed = pullRequest
        ? await refreshPullRequest(workspace.id, current, pullRequest)
        : current;
      return [
        workspace.id,
        {
          ...refreshed,
          status:
            workspace.status === "running"
              ? "running"
              : refreshed.done
                ? "done"
                : workspace.status
        }
      ] as const;
    })
  );
  return Object.fromEntries(entries);
}

function findPullRequest(
  workspace: WorkspaceRecord
): { readonly number: number; readonly url: string } | undefined {
  if (workspace.pullRequest) return workspace.pullRequest;
  for (let index = workspace.messages.length - 1; index >= 0; index -= 1) {
    const matches = workspace.messages[index]?.text.match(
      /https:\/\/github\.com\/vercel\/turborepo\/pull\/\d+/g
    );
    const url = matches?.at(-1);
    if (url) return { number: Number(url.split("/").at(-1)), url };
  }
}

async function refreshPullRequest(
  workspaceId: string,
  current: DisplayStatus,
  pullRequest: NonNullable<DisplayStatus["pullRequest"]>
): Promise<DisplayStatus> {
  if (
    pullRequest.state === "merged" ||
    (pullRequest.checkedAt &&
      Date.now() - Date.parse(pullRequest.checkedAt) < REFRESH_INTERVAL_MS)
  )
    return current.pullRequest ? current : { ...current, pullRequest };
  try {
    const response = await fetch(
      `https://api.github.com/repos/vercel/turborepo/pulls/${pullRequest.number}`,
      {
        headers: {
          accept: "application/vnd.github+json",
          authorization: `Bearer ${await getGitHubToken()}`,
          "x-github-api-version": "2022-11-28"
        }
      }
    );
    if (!response.ok) return current;
    const body = (await response.json()) as Record<string, unknown>;
    const state: PullRequestState | undefined =
      typeof body.merged_at === "string"
        ? "merged"
        : body.state === "open" || body.state === "closed"
          ? body.state
          : undefined;
    if (!state) return current;
    const next = {
      ...current,
      done: state === "merged",
      pullRequest: {
        number: pullRequest.number,
        url: pullRequest.url,
        checkedAt: new Date().toISOString(),
        state
      }
    };
    await writeStatus(workspaceId, next);
    return next;
  } catch {
    return current;
  }
}

async function readStatus(workspaceId: string): Promise<DisplayStatus> {
  const result = await get(`${PREFIX}${workspaceId}.json`, {
    access: "private",
    useCache: false
  });
  if (!result || result.statusCode !== 200) return {};
  const value: unknown = await new Response(result.stream).json().catch(() => null);
  return typeof value === "object" && value !== null ? (value as DisplayStatus) : {};
}

async function writeStatus(
  workspaceId: string,
  status: DisplayStatus
): Promise<void> {
  await put(`${PREFIX}${workspaceId}.json`, JSON.stringify(status), {
    access: "private",
    addRandomSuffix: false,
    allowOverwrite: true,
    contentType: "application/json"
  });
}
