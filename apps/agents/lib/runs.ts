import { put, list, get } from "@vercel/blob";

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

async function readBlobText(pathname: string): Promise<string | null> {
  const result = await get(pathname, { access: "private" });
  if (!result) return null;
  const reader = result.stream.getReader();
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  const combined = new Uint8Array(chunks.reduce((acc, c) => acc + c.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.length;
  }
  return new TextDecoder().decode(combined);
}

// In-memory cache so updateRun doesn't depend on list() consistency.
const runCache = new Map<string, RunMeta>();

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
    contentType: "application/json",
    addRandomSuffix: false
  });

  runCache.set(id, meta);
  return meta;
}

export async function updateRun(
  id: string,
  updates: Partial<Omit<RunMeta, "id" | "createdAt">>
): Promise<RunMeta> {
  let current = runCache.get(id);
  if (!current) {
    current = (await getRun(id)) ?? undefined;
  }
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
    addRandomSuffix: false,
    allowOverwrite: true
  });

  runCache.set(id, updated);
  return updated;
}

export async function getRun(id: string): Promise<RunMeta | null> {
  try {
    const text = await readBlobText(metaPath(id));
    if (!text) return null;
    return JSON.parse(text) as RunMeta;
  } catch {
    return null;
  }
}

export async function listRuns(limit = 20): Promise<RunMeta[]> {
  const { blobs } = await list({ prefix: RUNS_PREFIX });

  const metaBlobs = blobs.filter((b) => b.pathname.endsWith("meta.json"));

  metaBlobs.sort(
    (a, b) =>
      new Date(b.uploadedAt).getTime() - new Date(a.uploadedAt).getTime()
  );

  const runs: RunMeta[] = [];
  for (const blob of metaBlobs.slice(0, limit)) {
    try {
      const text = await readBlobText(blob.pathname);
      if (text) {
        runs.push(JSON.parse(text) as RunMeta);
      }
    } catch {
      // Skip corrupt entries
    }
  }

  return runs;
}

export async function appendLogs(id: string, lines: string): Promise<void> {
  let existing = "";
  try {
    existing = (await readBlobText(logsPath(id))) ?? "";
  } catch {
    // No existing log file yet
  }

  await put(logsPath(id), existing + lines, {
    access: "private",
    contentType: "text/plain",
    addRandomSuffix: false,
    allowOverwrite: true
  });
}

export async function getLogs(id: string): Promise<string> {
  try {
    return (await readBlobText(logsPath(id))) ?? "";
  } catch {
    return "";
  }
}
