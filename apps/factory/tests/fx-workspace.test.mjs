import assert from "node:assert/strict";
import test from "node:test";

import {
  countFxSessions,
  prepareFxInteractiveLaunch
} from "../agent/lib/fx-interactive.ts";
import {
  FX_TERMINAL_RUNNER_SOURCE,
  parseFxTerminalResult
} from "../agent/lib/fx-terminal-runner.ts";
import { workspaceNetworkPolicy } from "../agent/lib/workspace-network-policy.ts";

test("parseFxTerminalResult reads the completed interactive turn", () => {
  assert.deepEqual(
    parseFxTerminalResult(
      `diagnostic\n${JSON.stringify({ output: "done", sessionId: "session-123" })}\n`
    ),
    { output: "done", sessionId: "session-123" }
  );
  assert.equal(parseFxTerminalResult("not json"), null);
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
          return {
            exitCode: 0,
            async stdout() {
              return "not json";
            }
          };
        }
      },
      "/factory/turborepo"
    ),
    0
  );
});

test("prepareFxInteractiveLaunch attaches or resumes the shared fx terminal", async () => {
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
  assert.equal(writes[0].content.toString("utf8"), "oidc-token");
  assert.equal(launch.command, "bash");
  assert.match(launch.args[1], /tmux has-session/);
  assert.match(launch.args[1], /tmux attach-session/);
  assert.match(launch.args[1], /tmux new-session/);
  assert.match(launch.args[1], /fx --record resume --id/);
  assert.equal(launch.args[3], writes[0].path);
  assert.equal(launch.args[4], "session-123");
  assert.equal(launch.args[5], "factory-fx");
});

test("the terminal runner starts fx once and injects the autonomous prompt", () => {
  assert.match(FX_TERMINAL_RUNNER_SOURCE, /new-session/);
  assert.match(FX_TERMINAL_RUNNER_SOURCE, /respawn-pane/);
  assert.match(FX_TERMINAL_RUNNER_SOURCE, /paste-buffer/);
  assert.match(FX_TERMINAL_RUNNER_SOURCE, /fx --record/);
  assert.match(FX_TERMINAL_RUNNER_SOURCE, /\/factory\/bin:\$PATH/);
  assert.doesNotMatch(FX_TERMINAL_RUNNER_SOURCE, /session\/cancel/);
});

test("workspace network policy uses the Sandbox API custom wire format", () => {
  const policy = workspaceNetworkPolicy("github-token");
  assert.notEqual(typeof policy, "string");
  assert.deepEqual(policy.allowedDomains, [
    "api.github.com",
    "github.com",
    "*"
  ]);
  assert.deepEqual(policy.deniedCIDRs, ["169.254.169.254/32"]);
  assert.equal(policy.mode, "custom");
  assert.deepEqual(policy.injectionRules[1].headers, {
    authorization: `Basic ${Buffer.from("x-access-token:github-token").toString("base64")}`
  });
});

test("workspace publication credentials are injected only for the exact route", () => {
  const policy = workspaceNetworkPolicy("github-token", {
    authorization: "Bearer publish-token",
    hostname: "factory.example",
    path: "/api/workspaces/ws_abc/publish",
    url: "https://factory.example/api/workspaces/ws_abc/publish"
  });
  assert.notEqual(typeof policy, "string");
  assert.equal(policy.allowedDomains[0], "factory.example");
  assert.deepEqual(policy.injectionRules[0], {
    domain: "factory.example",
    headers: { authorization: "Bearer publish-token" },
    match: {
      method: ["POST"],
      path: "/api/workspaces/ws_abc/publish"
    }
  });
});
