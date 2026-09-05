import { existsSync, readdirSync } from "node:fs";
import path from "node:path";

export function listExamples(): string[] {
  const examplesRoot = path.resolve(process.cwd(), "../../examples");
  return readdirSync(examplesRoot, { withFileTypes: true })
    .filter(
      (entry) =>
        entry.isDirectory() &&
        existsSync(path.join(examplesRoot, entry.name, "package.json"))
    )
    .map((entry) => entry.name)
    .sort();
}
