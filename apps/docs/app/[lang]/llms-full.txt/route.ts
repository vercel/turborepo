import type { NextRequest } from "next/server";
import { cacheLife } from "next/cache";
import {
  geistdocsSource,
  openapiGeistdocsSource
} from "@/lib/geistdocs/source";

const getLlmsFull = async (lang: string) => {
  "use cache";
  cacheLife("max");

  const pages = [
    ...geistdocsSource.source
      .getPages(lang)
      .filter((page) => !page.url.includes("/acknowledgments"))
      .map((page) => ({ page, source: geistdocsSource })),
    ...openapiGeistdocsSource.source
      .getPages(lang)
      .map((page) => ({ page, source: openapiGeistdocsSource }))
  ];

  const scan = pages.map(async ({ page, source }) => {
    const processed = await source.getPageMarkdown(page);
    return `- [${page.data.title}](${page.url}): ${page.data.description ?? ""}

${processed}`;
  });

  const scanned = await Promise.all(scan);

  return scanned.join("\n\n");
};

export const GET = async (
  _req: NextRequest,
  { params }: RouteContext<"/[lang]/llms-full.txt">
) => {
  const { lang } = await params;

  return new Response(await getLlmsFull(lang), {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8"
    }
  });
};
