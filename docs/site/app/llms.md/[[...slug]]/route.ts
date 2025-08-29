// This file is mostly a copy-paste from https://fumadocs.vercel.app/docs/ui/llms.

import { notFound } from "next/navigation";
import { type NextRequest } from "next/server";
import { repoDocsPages } from "../../source";
import { parseFileContent, processMarkdownContent } from "../../lib/llms-utils";

export const revalidate = false;

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ slug?: Array<string> }> }
) {
  const { slug } = await params;
  const page = repoDocsPages.getPage(slug);
  if (!page) notFound();

  const { content } = await parseFileContent(page.data._file.absolutePath);
  const txt = await processMarkdownContent(content);
  return new Response(txt);
}

export function generateStaticParams() {
  return repoDocsPages.generateParams();
}
