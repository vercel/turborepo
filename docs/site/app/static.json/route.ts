import { NextResponse } from "next/server";
import type { DocumentRecord } from "fumadocs-core/search/algolia";
import { repoDocsPages } from "#app/source.ts";

export const revalidate = false;

export function GET(): NextResponse {
  const results: Array<DocumentRecord> = [];

  for (const page of [...repoDocsPages.getPages()]) {
    results.push({
      _id: page.url,
      structured: page.data.structuredData,
      url: page.url,
      title: page.data.title,
      description: page.data.description,
    });
  }

  return NextResponse.json(results);
}
