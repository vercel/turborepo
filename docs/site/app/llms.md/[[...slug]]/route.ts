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

  const { data, content } = await parseFileContent(
    page.data._file.absolutePath
  );
  const txt = await processMarkdownContent(content);

  const header = `# ${data.title}
Description: ${data.description}

`;

  return new Response(header.concat(txt));
}

export function generateStaticParams() {
  return repoDocsPages.generateParams();
}
