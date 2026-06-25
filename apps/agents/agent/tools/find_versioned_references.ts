import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { getExamplePath, getRepoRoot } from "../lib/repo.js";

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
  async execute({ example }) {
    const repoRoot = await getRepoRoot();
    const scanRoot = example
      ? await getExamplePath(example)
      : path.join(repoRoot, "examples");
    const files = await findTextFiles(scanRoot);
    const references: VersionReference[] = [];

    for (const file of files) {
      const content = await readFile(file, "utf8");
      const relativePath = path.relative(repoRoot, file);
      const lines = content.split("\n");
      lines.forEach((line, index) => {
        references.push(...findReferencesInLine(relativePath, index + 1, line));
      });
    }

    return { scannedFiles: files.length, references };
  }
});

async function findTextFiles(directory: string): Promise<string[]> {
  if (!existsSync(directory)) {
    return [];
  }

  const results: string[] = [];
  const entries = await readdir(directory, { withFileTypes: true });
  await Promise.all(
    entries.map(async (entry) => {
      if (skippedDirectories.has(entry.name) || skippedFiles.has(entry.name)) {
        return;
      }

      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        results.push(...(await findTextFiles(entryPath)));
        return;
      }

      if (!entry.isFile()) {
        return;
      }

      if (
        entry.name === "Dockerfile" ||
        textFileExtensions.has(path.extname(entry.name))
      ) {
        results.push(entryPath);
      }
    })
  );
  return results.sort();
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
