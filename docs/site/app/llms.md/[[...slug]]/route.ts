// This file is mostly a copy-paste from https://fumadocs.vercel.app/docs/ui/llms.

import * as fs from "node:fs/promises";
import { notFound } from "next/navigation";
import { type NextRequest, NextResponse } from "next/server";
import { repoDocsPages } from "../../source";

export const revalidate = false;

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ slug?: Array<string> }> }
) {
  const { slug } = await params;
  const page = repoDocsPages.getPage(slug);
  if (!page) notFound();

  const fileContent = await fs.readFile(page.data._file.absolutePath);
  return new NextResponse(fileContent);
}

export function generateStaticParams() {
  return repoDocsPages.generateParams();
}
