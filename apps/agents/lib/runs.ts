import { put, list } from "@vercel/blob";

export type RunStatus =
  | "queued"
  | "scanning"
  | "fixing"
  | "pushing"
  | "completed"
  | "failed";

export interface RunMeta {
  id: string;
  status: RunStatus;
  trigger: "cron" | "manual";
  createdAt: string;
  updatedAt: string;
  sandboxId?: string;
  branch?: string;
  error?: string;
  vulnerabilities?: { cargo: number; pnpm: number };
  agentResults?: {
    success: boolean;
    summary: string;
    vulnerabilitiesFixed: number;
    vulnerabilitiesRemaining: number;
  };
  diffUrl?: string;
}

const RUNS_PREFIX = "runs/";
const LOGS_PREFIX = "logs/";

function metaPath(id: string): string {
  return `${RUNS_PREFIX}${id}/meta.json`;
}

function logsPath(id: string): string {
  return `${LOGS_PREFIX}${id}.log`;
}

export async function createRun(trigger: "cron" | "manual"): Promise<RunMeta> {
  const id = `run-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const now = new Date().toISOString();
  const meta: RunMeta = {
    id,
    status: "queued",
    trigger,
    createdAt: now,
    updatedAt: now
  };

  await put(metaPath(id), JSON.stringify(meta), {
    access: "private",
    contentType: "application/json"
  });

  // Initialize empty log file (use a space — the SDK rejects empty strings as falsy)
  await put(logsPath(id), " ", {
    access: "private",
    contentType: "text/plain"
  });

  return meta;
}

export async function updateRun(
  id: string,
  updates: Partial<Omit<RunMeta, "id" | "createdAt">>
): Promise<RunMeta> {
  const current = await getRun(id);
  if (!current) {
    throw new Error(`Run ${id} not found`);
  }

  const updated: RunMeta = {
    ...current,
    ...updates,
    updatedAt: new Date().toISOString()
  };

  await put(metaPath(id), JSON.stringify(updated), {
    access: "private",
    contentType: "application/json",
    addRandomSuffix: false
  });

  return updated;
}

export async function getRun(id: string): Promise<RunMeta | null> {
  const { blobs } = await list({ prefix: `${RUNS_PREFIX}${id}/meta` });
  const blob = blobs[0];
  if (!blob) return null;

  const result = await get(blob.url, { access: "private" });
  if (!result) return null;
  const text = await new Response(result.stream).text();
  return JSON.parse(text) as RunMeta;
}

export async function listRuns(limit = 20): Promise<RunMeta[]> {
  const { blobs } = await list({ prefix: RUNS_PREFIX });

  // Filter to only meta.json files
  const metaBlobs = blobs.filter((b) => b.pathname.endsWith("meta.json"));

  // Sort by upload date descending
  metaBlobs.sort(
    (a, b) =>
      new Date(b.uploadedAt).getTime() - new Date(a.uploadedAt).getTime()
  );

  const runs: RunMeta[] = [];
  for (const blob of metaBlobs.slice(0, limit)) {
    try {
      const result = await get(blob.url, { access: "private" });
      if (result) {
        const text = await new Response(result.stream).text();
        runs.push(JSON.parse(text) as RunMeta);
      }
    } catch {
      // Skip corrupt entries
    }
  }

  return runs;
}

export async function appendLogs(id: string, lines: string): Promise<void> {
  // Vercel Blob is append-unfriendly, so we read + append + rewrite.
  // For large logs this isn't ideal, but it's simple and works without a DB.
  const { blobs } = await list({ prefix: `${LOGS_PREFIX}${id}` });
  const blob = blobs[0];

  let existing = "";
  if (blob) {
    const result = await get(blob.url, { access: "private" });
    if (result) {
      existing = (await new Response(result.stream).text()).trimStart();
    }
  }

  await put(logsPath(id), existing + lines || " ", {
    access: "private",
    contentType: "text/plain",
    addRandomSuffix: false
  });
}

export async function getLogs(id: string): Promise<string> {
  const { blobs } = await list({ prefix: `${LOGS_PREFIX}${id}` });
  const blob = blobs[0];
  if (!blob) return "";

  const result = await get(blob.url, { access: "private" });
  if (!result) return "";
  return (await new Response(result.stream).text()).trimStart();
}
