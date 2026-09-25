import { execFileSync } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

const root = process.cwd();
const turbo = join(root, "node_modules/.bin/turbo");
const run = (args: string[]) =>
  execFileSync(turbo, args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, CI: "1" }
  });

test("building web schedules the UI build first", () => {
  const plan = JSON.parse(run(["run", "build", "--filter=web", "--dry=json"]));
  const web = plan.tasks.find(
    (task: { taskId: string }) => task.taskId === "web#build"
  );
  expect(web).toBeDefined();
  expect(web.dependencies).toContain("@repo/ui#build");
  expect(plan.tasks.map((task: { taskId: string }) => task.taskId)).toContain(
    "@repo/ui#build"
  );
  expect(web.outputs).toContain("dist/**");
});

test("a clean web build produces an artifact from the library", () => {
  rmSync(join(root, "apps/web/dist"), { recursive: true, force: true });
  rmSync(join(root, "packages/ui/dist"), { recursive: true, force: true });
  run(["run", "build", "--filter=web", "--force", "--ui=stream"]);
  expect(readFileSync(join(root, "apps/web/dist/message.txt"), "utf8")).toBe(
    "Hello from UI\n"
  );
});
