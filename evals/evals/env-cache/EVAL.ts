import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

const root = process.cwd();
const turbo = join(root, "node_modules/.bin/turbo");
type Task = { taskId: string; hash: string; outputs: string[] };
function plan(origin: string): Map<string, Task> {
  const result = execFileSync(turbo, ["run", "build", "--dry=json"], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, CI: "1", STOREFRONT_API_ORIGIN: origin }
  });
  return new Map(
    (JSON.parse(result).tasks as Task[]).map((task) => [task.taskId, task])
  );
}

test("only web's build hash changes with STOREFRONT_API_ORIGIN", () => {
  const preview = plan("https://preview.example.test");
  const production = plan("https://production.example.test");
  expect(preview.get("web#build")?.hash).not.toBe(
    production.get("web#build")?.hash
  );
  expect(preview.get("@repo/ui#build")?.hash).toBe(
    production.get("@repo/ui#build")?.hash
  );
  expect(preview.get("web#build")?.outputs).toContain("dist/**");
});

test("the web build sees the configured origin", () => {
  execFileSync(
    turbo,
    ["run", "build", "--filter=web", "--force", "--ui=stream"],
    {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        CI: "1",
        STOREFRONT_API_ORIGIN: "https://production.example.test"
      }
    }
  );
  expect(readFileSync(join(root, "apps/web/dist/api-origin.txt"), "utf8")).toBe(
    "https://production.example.test"
  );
});
