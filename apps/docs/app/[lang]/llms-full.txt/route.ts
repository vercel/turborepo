import type { NextRequest } from "next/server";
import { getLLMText, source } from "@/lib/geistdocs/source";

export const revalidate = false;

export const GET = async (
  _req: NextRequest,
  { params }: RouteContext<"/[lang]/llms-full.txt">
) => {
  const { lang } = await params;
  const pages = source
    .getPages(lang)
    .filter((page) => !page.url.includes("/acknowledgments"));

  const scan = pages.map(async (page) => {
    const processed = await getLLMText(page);
    return `- [${page.data.title}](${page.url}): ${page.data.description ?? ""}

${processed}`;
  });

  const scanned = await Promise.all(scan);

  return new Response(scanned.join("\n\n"));
};
