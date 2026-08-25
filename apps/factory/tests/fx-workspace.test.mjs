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
import { requireFactoryImage } from "../agent/lib/current-factory-image.ts";
import {
  applyWorkspaceNetworkPolicy,
  configureWorkspaceGitAuthentication,
  workspaceNetworkPolicy
} from "../agent/lib/workspace-network-policy.ts";

test("workspace creation uses any published factory image", () => {
  const pointer = {
    buildId: "build-123",
    commit: "0123456789abcdef0123456789abcdef01234567",
    fingerprint: "older-image",
    publishedAt: "2026-08-24T00:00:00.000Z",
    sandboxName: "factory-image-test",
    snapshotId: "snap_123",
    warmBuild: true
  };
  assert.equal(requireFactoryImage(pointer), pointer);
  assert.throws(() => requireFactoryImage(null), /has been published/);
});

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

test("workspace policy uses the Sandbox API custom schema", async () => {
  const updates = [];
  const policy = workspaceNetworkPolicy("github-token");
  await applyWorkspaceNetworkPolicy(
    {
      currentSession() {
        return {
          async update(input) {
            updates.push(input);
          }
        };
      }
    },
    policy
  );

  assert.equal(policy.mode, "custom");
  assert.deepEqual(policy.allowedDomains, [
    "api.github.com",
    "github.com",
    "*"
  ]);
  assert.deepEqual(policy.deniedCIDRs, ["169.254.169.254/32"]);
  assert.deepEqual(updates, [{ networkPolicy: policy }]);
});

test("workspace publication credentials are injected only for the exact route", () => {
  const policy = workspaceNetworkPolicy("github-token", {
    authorization: "[redacted]",
    hostname: "factory.example",
    path: "/api/workspaces/ws_abc/publish",
    url: "https://factory.example/api/workspaces/ws_abc/publish"
  });
  assert.equal(policy.allowedDomains[0], "factory.example");
  assert.deepEqual(policy.injectionRules[0], {
    domain: "factory.example",
    headers: { authorization: "[redacted]" },
    match: {
      method: ["POST"],
      path: { exact: "/api/workspaces/ws_abc/publish" }
    }
  });
});

test("workspace Git sends a placeholder credential for policy replacement", async () => {
  const commands = [];
  await configureWorkspaceGitAuthentication({
    async runCommand(command) {
      commands.push(command);
      return {
        exitCode: 0,
        async stderr() {
          return "";
        }
      };
    }
  });

  assert.equal(commands.length, 1);
  assert.equal(commands[0].cmd, "git");
  assert.equal(commands[0].cwd, "/factory/turborepo");
  assert.deepEqual(commands[0].args.slice(0, 3), [
    "config",
    "--local",
    "http.https://github.com/.extraheader"
  ]);
  assert.match(commands[0].args[3], /^Authorization: Basic /);
  assert.doesNotMatch(commands[0].args[3], /github-token/);
});
