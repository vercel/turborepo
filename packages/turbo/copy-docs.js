const { cpSync, copyFileSync, rmSync } = require("node:fs");
const path = require("node:path");

const packageRoot = __dirname;
const repositoryRoot = path.resolve(packageRoot, "../..");
const sourceDocs = path.join(repositoryRoot, "apps/docs/content/docs");
const packageDocs = path.join(packageRoot, "docs");

rmSync(packageDocs, { recursive: true, force: true });
cpSync(sourceDocs, packageDocs, { recursive: true });
copyFileSync(
  path.join(packageRoot, "docs-index.md"),
  path.join(packageDocs, "README.md")
);
