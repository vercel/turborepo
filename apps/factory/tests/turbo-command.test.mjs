import assert from "node:assert/strict";
import test from "node:test";

import { buildTurboRunCommand } from "../agent/lib/turbo-command.ts";

const tasks = ["build", "lint"];

test("builds package-manager-specific Turbo commands", () => {
  assert.deepEqual(buildTurboRunCommand("pnpm", tasks), {
    command: "pnpm",
    args: ["exec", "turbo", "run", ...tasks, "--continue=always"],
    tasks
  });
  assert.deepEqual(buildTurboRunCommand("npm", tasks), {
    command: "npm",
    args: ["exec", "turbo", "--", "run", ...tasks, "--continue=always"],
    tasks
  });
  assert.deepEqual(buildTurboRunCommand("yarn", tasks), {
    command: "yarn",
    args: ["exec", "turbo", "run", ...tasks, "--continue=always"],
    tasks
  });
  assert.deepEqual(buildTurboRunCommand("bun", tasks), {
    command: "bunx",
    args: ["turbo", "run", ...tasks, "--continue=always"],
    tasks
  });
});

test("deduplicates task names", () => {
  assert.deepEqual(buildTurboRunCommand("pnpm", ["build", "build"]).tasks, [
    "build"
  ]);
});

test("rejects empty and flag-like task names", () => {
  assert.throws(() => buildTurboRunCommand("pnpm", []));
  assert.throws(() => buildTurboRunCommand("pnpm", [""]));
  assert.throws(() => buildTurboRunCommand("pnpm", ["--filter=web"]));
});
