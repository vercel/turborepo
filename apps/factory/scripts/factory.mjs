#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const baseUrl = process.env.FACTORY_URL;
const bypass = process.env.VERCEL_AUTOMATION_BYPASS_SECRET;
const [command, ...args] = process.argv.slice(2);

if (!baseUrl) fail("FACTORY_URL is required.");

function url(path) {
  return new URL(path, baseUrl);
}

async function request(path, options = {}) {
  const target = url(path);
  const response = await fetch(target, {
    ...options,
    headers: {
      ...(options.body ? { "content-type": "application/json" } : {}),
      ...(options.action ? { "x-operator-action": options.action } : {}),
      ...(bypass ? { "x-vercel-protection-bypass": bypass } : {}),
      origin: target.origin
    }
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) fail(body.error ?? `Factory returned ${response.status}.`);
  return body;
}

if (command === "list") {
  const { workspaces } = await request("/api/workspaces");
  for (const workspace of workspaces) {
    console.log(`${workspace.id}\t${workspace.status}\t${workspace.title}`);
  }
} else if (command === "start") {
  const prompt = args.join(" ").trim();
  if (!prompt) fail('Usage: factory start "<prompt>"');
  const workspace = await request("/api/workspaces", {
    action: "create-workspace",
    body: JSON.stringify({ prompt }),
    method: "POST"
  });
  console.log(new URL(`/workspaces/${workspace.id}`, baseUrl).toString());
} else if (command === "ssh") {
  const [id] = args;
  if (!id) fail("Usage: factory ssh <workspace-id>");
  const workspace = await request(`/api/workspaces/${encodeURIComponent(id)}`);
  if (workspace.chatCommand) {
    console.log(`Resume chat after connecting: ${workspace.chatCommand}`);
  }
  const result = spawnSync(
    "sandbox",
    ["ssh", workspace.sandbox.name],
    { stdio: "inherit" }
  );
  process.exit(result.status ?? 1);
} else {
  fail("Usage: factory <list|start|ssh> [arguments]");
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
