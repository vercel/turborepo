import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { getExamplePath, getRepoRoot, listTrackedFiles } from "../lib/repo.js";

interface VersionReference {
  path: string;
  line: number;
  kind: "docker-image" | "github-action" | "package-manager" | "node-version";
  value: string;
}

const textFileExtensions = new Set([
  ".dockerfile",
  ".json",
  ".jsonc",
  ".md",
  ".mdx",
  ".ts",
  ".tsx",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
  ".yaml",
  ".yml"
]);
const skippedDirectories = new Set([
  ".git",
  ".next",
  ".turbo",
  "dist",
  "node_modules"
]);
const skippedFiles = new Set([
  "package-lock.json",
  "pnpm-lock.yaml",
  "yarn.lock"
]);

export default defineTool({
  description:
    "Find versioned references in example files outside package metadata, including Docker image tags, GitHub Actions versions, package-manager pins, and Node version mentions.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .optional()
      .describe(
        "Optional directory name under examples/. Omit to scan every example."
      )
  }),
  async execute({ example }, ctx) {
    const sandbox = await ctx.getSandbox();
    const repoRoot = await getRepoRoot(sandbox);
    const scanRoot = example
      ? await getExamplePath(sandbox, example)
      : path.join(repoRoot, "examples");
    const relativeRoot = path.relative(repoRoot, scanRoot);
    const files = (await listTrackedFiles(sandbox, relativeRoot))
      .filter(isTextFile)
      .map((file) => path.join(repoRoot, file));
    const references: VersionReference[] = [];

    for (const file of files) {
      const content = await sandbox.readTextFile({ path: file });
      if (content === null) {
        continue;
      }
      const relativePath = path.relative(repoRoot, file);
      const lines = content.split("\n");
      for (const [index, line] of lines.entries()) {
        references.push(...findReferencesInLine(relativePath, index + 1, line));
      }
    }

    return { scannedFiles: files.length, references };
  }
});

function isTextFile(file: string): boolean {
  const parts = file.split("/");
  return (
    !parts.some((part) => skippedDirectories.has(part)) &&
    !skippedFiles.has(path.basename(file)) &&
    (path.basename(file) === "Dockerfile" ||
      textFileExtensions.has(path.extname(file)))
  );
}

function findReferencesInLine(
  filePath: string,
  lineNumber: number,
  line: string
): VersionReference[] {
  const references: VersionReference[] = [];
  const fromMatch = line.match(
    /^\s*FROM\s+([^\s:@]+(?:\/[^\s:@]+)*):([^\s@]+)/i
  );
  if (fromMatch?.[1] && fromMatch[2]) {
    references.push({
      path: filePath,
      line: lineNumber,
      kind: "docker-image",
      value: `${fromMatch[1]}:${fromMatch[2]}`
    });
  }

  for (const match of line.matchAll(/uses:\s*([\w./-]+)@([^\s#]+)/g)) {
    references.push({
      path: filePath,
      line: lineNumber,
      kind: "github-action",
      value: `${match[1]}@${match[2]}`
    });
  }

  for (const match of line.matchAll(/\b(pnpm|npm|yarn)@(\d+[^\s`'"),]*)/g)) {
    references.push({
      path: filePath,
      line: lineNumber,
      kind: "package-manager",
      value: `${match[1]}@${match[2]}`
    });
  }

  for (const match of line.matchAll(
    /\bnode(?:\.js)?\s*(?:version)?\s*[>=:^~ ]+v?(\d+(?:\.\d+){0,2})/gi
  )) {
    references.push({
      path: filePath,
      line: lineNumber,
      kind: "node-version",
      value: match[1] ?? ""
    });
  }

  return references;
}
