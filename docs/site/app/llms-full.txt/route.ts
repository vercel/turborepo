// This file is mostly a copy-paste from https://fumadocs.vercel.app/docs/ui/llms.

import {
  scanDocumentationFiles,
  parseFileContent,
  processMarkdownContent,
  formatFilePath,
} from "../lib/llms-utils";

export const revalidate = false;

export async function GET(): Promise<Response> {
  // all scanned content
  const files = await scanDocumentationFiles();

  const scan = files.map(async (file) => {
    const { content, data } = await parseFileContent(file);

    const processed = await processMarkdownContent(content);
    return `- [${data.title}](${formatFilePath(file)}): ${data.description}

${processed}`;
  });

  const scanned = await Promise.all(scan);

  return new Response(scanned.join("\n\n"));
}
