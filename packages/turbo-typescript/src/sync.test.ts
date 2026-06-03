import {
  mkdtemp,
  readFile,
  rm,
  writeFile,
  mkdir,
  symlink
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, expect, test } from "@jest/globals";
import {
  checkProjectReferences,
  getProjectReferenceCandidates,
  writeProjectReferences
} from "./sync";

const roots: Array<string> = [];

afterEach(async () => {
  await Promise.all(
    roots.map((root) => rm(root, { recursive: true, force: true }))
  );
  roots.length = 0;
});

test("write converges root and package references", async () => {
  const root = await fixture({
    "turbo.json": JSON.stringify(
      { typescriptProjectReferences: true, tasks: {} },
      null,
      2
    ),
    "package.json": JSON.stringify(
      {
        packageManager: "npm@10.0.0",
        workspaces: ["packages/*"],
        devDependencies: { typescript: "5.5.4" }
      },
      null,
      2
    ),
    "tsconfig.json": JSON.stringify(
      { include: ["packages"], compilerOptions: { strict: true } },
      null,
      2
    ),
    "packages/ui/package.json": JSON.stringify(
      { name: "ui", version: "1.0.0" },
      null,
      2
    ),
    "packages/ui/tsconfig.json": JSON.stringify(
      { compilerOptions: { declaration: true } },
      null,
      2
    ),
    "packages/web/package.json": JSON.stringify(
      { name: "web", version: "1.0.0", dependencies: { ui: "^1.0.0" } },
      null,
      2
    ),
    "packages/web/tsconfig.json": JSON.stringify({}, null, 2)
  });

  const result = await writeProjectReferences({ cwd: root });

  expect(result.success).toBe(true);
  expect(result.summary.validCount).toBe(2);
  expect(
    JSON.parse(await readFile(path.join(root, "tsconfig.json"), "utf8"))
  ).toEqual({
    compilerOptions: { strict: true },
    files: [],
    references: [{ path: "packages/ui" }, { path: "packages/web" }]
  });
  expect(
    JSON.parse(
      await readFile(path.join(root, "packages/web/tsconfig.json"), "utf8")
    )
  ).toEqual({
    references: [{ path: "../ui" }],
    compilerOptions: { composite: true }
  });
});

test("write ignores packages without tsconfig and check reports drift", async () => {
  const root = await fixture({
    "turbo.json": JSON.stringify(
      {
        typescriptProjectReferences: { excluded: ["packages/app"] },
        tasks: {}
      },
      null,
      2
    ),
    "package.json": JSON.stringify(
      {
        packageManager: "npm@10.0.0",
        workspaces: ["packages/*"],
        devDependencies: { typescript: "5.5.4" }
      },
      null,
      2
    ),
    "tsconfig.json": JSON.stringify({ files: [], references: [] }, null, 2),
    "packages/app/package.json": JSON.stringify(
      { name: "app", version: "1.0.0", dependencies: { config: "^1.0.0" } },
      null,
      2
    ),
    "packages/app/tsconfig.json": JSON.stringify({}, null, 2),
    "packages/config/package.json": JSON.stringify(
      { name: "config", version: "1.0.0" },
      null,
      2
    )
  });

  const check = await checkProjectReferences({ cwd: root });
  expect(check.success).toBe(false);

  const write = await writeProjectReferences({ cwd: root });
  expect(write.summary.ignoredCount).toBe(1);
  expect(
    JSON.parse(await readFile(path.join(root, "turbo.json"), "utf8"))
  ).toMatchObject({
    typescriptProjectReferences: { ignored: ["packages/config"] }
  });
});

test("candidates lists excluded packages that are now valid", async () => {
  const root = await fixture({
    "turbo.json": JSON.stringify(
      { typescriptProjectReferences: { excluded: ["packages/ui"] }, tasks: {} },
      null,
      2
    ),
    "package.json": JSON.stringify(
      {
        packageManager: "npm@10.0.0",
        workspaces: ["packages/*"],
        devDependencies: { typescript: "5.5.4" }
      },
      null,
      2
    ),
    "tsconfig.json": JSON.stringify({ files: [], references: [] }, null, 2),
    "packages/ui/package.json": JSON.stringify(
      { name: "ui", version: "1.0.0" },
      null,
      2
    ),
    "packages/ui/tsconfig.json": JSON.stringify({}, null, 2)
  });

  const result = await getProjectReferenceCandidates({ cwd: root });

  expect(result.success).toBe(true);
  expect(result.candidates).toEqual(["packages/ui"]);
});

async function fixture(files: Record<string, string>): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), "turbo-typescript-"));
  roots.push(root);
  const typescriptPath = path.dirname(
    require.resolve("typescript/package.json")
  );
  await mkdir(path.join(root, "node_modules"), { recursive: true });
  await symlink(
    typescriptPath,
    path.join(root, "node_modules/typescript"),
    "dir"
  );
  for (const [relativePath, contents] of Object.entries(files)) {
    const filePath = path.join(root, relativePath);
    await mkdir(path.dirname(filePath), { recursive: true });
    await writeFile(filePath, `${contents}\n`);
  }
  return root;
}
