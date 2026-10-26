import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

const root = process.cwd();
const turbo = join(root, "node_modules/.bin/turbo");

function command(binary: string, args: string[]) {
  const result = spawnSync(binary, args, {
    cwd: root,
    encoding: "utf8",
    timeout: 30_000,
    env: {
      ...process.env,
      CI: "1",
      TURBO_TELEMETRY_DISABLED: "1",
      DOCS_PREVIEW_TOKEN: ""
    }
  });
  if (result.error) throw result.error;
  return result;
}

function plan(task: string) {
  const result = command(turbo, ["run", task, "--dry=json"]);
  expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
  const parsed: unknown = JSON.parse(result.stdout);
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    !("tasks" in parsed) ||
    !Array.isArray(parsed.tasks)
  ) {
    throw new Error("Turbo dry run must return a tasks array");
  }
  return parsed.tasks.map((task: unknown) => {
    if (
      typeof task !== "object" ||
      task === null ||
      !("taskId" in task) ||
      typeof task.taskId !== "string" ||
      !("command" in task) ||
      typeof task.command !== "string" ||
      !("dependencies" in task) ||
      !Array.isArray(task.dependencies)
    ) {
      throw new Error(
        "Turbo dry run task must have taskId, command, and dependencies"
      );
    }
    const dependencies = task.dependencies.map((dependency: unknown) => {
      if (typeof dependency !== "string") {
        throw new Error("Turbo task dependencies must be task IDs");
      }
      return dependency;
    });
    return { taskId: task.taskId, command: task.command, dependencies };
  });
}

function expectTestPlan(tasks: ReturnType<typeof plan>) {
  const runnable = tasks.filter((task) => task.command !== "<NONEXISTENT>");
  expect(
    runnable.map((task) => task.taskId).sort(),
    "test must run only web tests and their required web/UI builds"
  ).toEqual(["@repo/ui#build", "web#build", "web#test"]);
  expect(
    tasks.find((task) => task.taskId === "web#test")?.dependencies
  ).toContain("web#build");
  expect(
    tasks.find((task) => task.taskId === "web#build")?.dependencies
  ).toContain("@repo/ui#build");
}

function cleanBuilds() {
  for (const directory of [
    ".turbo",
    "apps/web/dist",
    "apps/docs/dist",
    "packages/ui/dist"
  ]) {
    rmSync(join(root, directory), { recursive: true, force: true });
  }
}

test("test selects only executable tests and preserves their build dependencies", () => {
  expectTestPlan(plan("test"));
}, 60_000);

test("a newly added test-less workspace does not pull its build into test", () => {
  const lockPath = join(root, "package-lock.json");
  const originalLock = readFileSync(lockPath);
  const admin = mkdtempSync(join(root, "apps/admin-"));
  try {
    writeFileSync(
      join(admin, "package.json"),
      JSON.stringify(
        {
          name: "admin",
          version: "1.0.0",
          private: true,
          scripts: { build: "node build.cjs" }
        },
        null,
        2
      ) + "\n"
    );
    writeFileSync(
      join(admin, "build.cjs"),
      'throw new Error("Admin preview cannot be built in the test job");\n'
    );
    // Register the new local workspace without fetching or changing installed tools.
    const install = command("npm", [
      "install",
      "--package-lock-only",
      "--offline",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund"
    ]);
    expect(install.status, `${install.stdout}\n${install.stderr}`).toBe(0);
    expectTestPlan(plan("test"));
    expect(
      plan("build").map((task) => task.taskId),
      "admin must be a discovered workspace"
    ).toContain("admin#build");
  } finally {
    rmSync(admin, { recursive: true, force: true });
    writeFileSync(lockPath, originalLock);
  }
}, 90_000);

test("a clean test run builds the UI and web without a docs token", () => {
  cleanBuilds();
  const result = command(turbo, ["run", "test", "--force", "--ui=stream"]);
  expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
  expect(result.stdout).toContain("Storefront renders the catalog button");
  expect(readFileSync(join(root, "apps/web/dist/index.html"), "utf8")).toBe(
    '<main><h1>Storefront</h1><button type="button">Browse catalog</button></main>\n'
  );
}, 60_000);

test("normal build still selects docs and requires its preview token", () => {
  const runnable = plan("build").filter(
    (task) => task.command !== "<NONEXISTENT>"
  );
  expect(
    runnable.map((task) => task.taskId),
    "normal build must retain docs"
  ).toContain("docs#build");
  cleanBuilds();
  const result = command(turbo, ["run", "build", "--force", "--ui=stream"]);
  expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(1);
  expect(`${result.stdout}\n${result.stderr}`).toContain(
    "DOCS_PREVIEW_TOKEN is required to build the docs preview"
  );
}, 60_000);
