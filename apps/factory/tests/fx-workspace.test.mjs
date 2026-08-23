import assert from "node:assert/strict";
import test from "node:test";

import { parseFxAcpResult } from "../agent/lib/fx-acp.ts";
import {
  cancelFxAcpTurn,
  countFxSessions,
  prepareFxInteractiveLaunch
} from "../agent/lib/fx-interactive.ts";
import { workspaceNetworkPolicy } from "../agent/lib/workspace-network-policy.ts";

test("parseFxAcpResult reads the final ACP client result", () => {
  assert.deepEqual(
    parseFxAcpResult(
      `diagnostic\n${JSON.stringify({ cancelled: true, output: "done", sessionId: "session-123" })}\n`
    ),
    { cancelled: true, output: "done", sessionId: "session-123" }
  );
  assert.equal(parseFxAcpResult("not json"), null);
});

test("cancelFxAcpTurn writes the ACP handoff signal", async () => {
  const writes = [];
  await cancelFxAcpTurn({
    async writeFiles(files) {
      writes.push(...files);
    }
  });
  assert.equal(writes.length, 1);
  assert.equal(writes[0].path, "/factory/state/fx-acp-cancel");
  assert.equal(writes[0].content.toString("utf8"), "cancel\n");
});

test("countFxSessions returns the current workspace session count", async () => {
  const calls = [];
  const count = await countFxSessions(
    {
      async runCommand(options) {
        calls.push(options);
        return {
          exitCode: 0,
          async stdout() {
            return JSON.stringify({ kind: "sessions", count: 2, sessions: [] });
          }
        };
      }
    },
    "/factory/turborepo"
  );

  assert.equal(count, 2);
  assert.deepEqual(calls, [
    {
      args: ["sessions", "--json", "--limit", "100"],
      cmd: "fx",
      cwd: "/factory/turborepo",
      timeoutMs: 10_000
    }
  ]);
});

test("countFxSessions treats invalid output as no sessions", async () => {
  assert.equal(
    await countFxSessions(
      {
        async runCommand() {
          return { exitCode: 0, async stdout() { return "not json"; } };
        }
      },
      "/factory/turborepo"
    ),
    0
  );
});

test("prepareFxInteractiveLaunch resumes the workspace session with OIDC", async () => {
  const writes = [];
  const launch = await prepareFxInteractiveLaunch(
    {
      async writeFiles(files) {
        writes.push(...files);
      }
    },
    "session-123",
    async () => "oidc-token"
  );

  assert.equal(writes.length, 1);
  assert.equal(writes[0].content.toString("utf8"), "oidc-token");
  assert.match(writes[0].path, /^\/factory\/state\/interactive-oidc-/);
  assert.equal(launch.command, "bash");
  assert.equal(launch.args[0], "-lc");
  assert.match(launch.args[1], /exec fx --yolo --resume/);
  assert.equal(launch.args[2], "factory-terminal");
  assert.equal(launch.args[3], writes[0].path);
  assert.equal(launch.args[4], "session-123");
});

test("workspace GitHub credential rules precede the catch-all rule", () => {
  const policy = workspaceNetworkPolicy("github-token");
  assert.notEqual(typeof policy, "string");
  assert.ok(policy.allow && !Array.isArray(policy.allow));

  assert.deepEqual(Object.keys(policy.allow), [
    "api.github.com",
    "github.com",
    "*"
  ]);
  assert.deepEqual(policy.allow["github.com"][0].transform, [
    {
      headers: {
        authorization: `Basic ${Buffer.from("x-access-token:github-token").toString("base64")}`
      }
    }
  ]);
});
