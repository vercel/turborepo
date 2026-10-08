import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

const root = process.cwd();
const turbo = join(root, "node_modules/.bin/turbo");
const configPath = join(root, "build-settings.json");

type Task = {
  taskId: string;
  hash: string;
  inputs: Record<string, string>;
  outputs: string[];
};
function taskHashes(): Map<string, Task> {
  const output = execFileSync(turbo, ["run", "build", "--dry=json"], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, CI: "1" }
  });
  return new Map(
    (JSON.parse(output).tasks as Task[]).map((task) => [task.taskId, task])
  );
}

test("root build-settings.json invalidates web but not UI", () => {
  const original = readFileSync(configPath, "utf8");
  try {
    const before = taskHashes();
    writeFileSync(configPath, JSON.stringify({ title: "Updated title" }));
    const after = taskHashes();
    expect(before.get("web#build")?.hash).not.toBe(
      after.get("web#build")?.hash
    );
    expect(before.get("@repo/ui#build")?.hash).toBe(
      after.get("@repo/ui#build")?.hash
    );
    expect(after.get("web#build")?.outputs).toContain("dist/**");
  } finally {
    writeFileSync(configPath, original);
  }
});

test("ordinary web source files still affect the build hash", () => {
  const source = join(root, "apps/web/build.cjs");
  const original = readFileSync(source, "utf8");
  try {
    const before = taskHashes();
    writeFileSync(source, `${original}\n// A code change\n`);
    const after = taskHashes();
    expect(before.get("web#build")?.hash).not.toBe(
      after.get("web#build")?.hash
    );
  } finally {
    writeFileSync(source, original);
  }
});
