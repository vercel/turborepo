// This file is mostly a copy-paste from https://fumadocs.vercel.app/docs/ui/llms.

import {
  scanDocumentationFiles,
  parseFileContent,
  formatFilePath,
} from "../lib/llms-utils";
import { PRODUCT_SLOGANS } from "../../lib/constants";

export const revalidate = false;

export async function GET(): Promise<Response> {
  // all scanned content
  const files = await scanDocumentationFiles();

  const scan = files.sort().map(async (file) => {
    const { data } = await parseFileContent(file);

    return `- [${data.title}](${formatFilePath(file)}): ${data.description}`;
  });

  const scanned = await Promise.all(scan);

  const header = `
# Turborepo documentation

Generated at: ${new Date().toUTCString()}

## Turborepo

> ${PRODUCT_SLOGANS.turbo}

## Docs

`;

  return new Response(header.concat(scanned.join("\n")));
}
