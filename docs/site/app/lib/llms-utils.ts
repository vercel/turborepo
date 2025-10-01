import * as fs from "node:fs/promises";
import fg from "fast-glob";
import matter from "gray-matter";
import { remark } from "remark";
import remarkStringify from "remark-stringify";
import remarkMdx from "remark-mdx";

export const DEFAULT_IGNORED_FILES = [
  "!./content/docs/acknowledgments.mdx",
  "!./content/docs/community.mdx",
  "!./content/docs/telemetry.mdx",
];

export async function scanDocumentationFiles(
  patterns: Array<string> = ["./content/docs/**/*.mdx"],
  ignorePatterns: Array<string> = DEFAULT_IGNORED_FILES
) {
  return fg([...patterns, ...ignorePatterns]);
}

export async function parseFileContent(filePath: string) {
  const fileContent = await fs.readFile(filePath);
  return matter(fileContent.toString());
}

export async function processMarkdownContent(content: string): Promise<string> {
  const file = await remark()
    .use(remarkMdx)
    .use(remarkStringify)
    .process(content);

  return String(file);
}

export function formatFilePath(filePath: string): string {
  return filePath.replace("./content/docs", "").replace(/\.mdx$/, ".md");
}
