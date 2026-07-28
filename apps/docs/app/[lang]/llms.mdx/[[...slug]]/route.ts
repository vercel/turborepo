import { createDocsMarkdownRoute } from "@vercel/geistdocs/routes/llms";
import { geistdocsSource } from "@/lib/geistdocs/source";

export const { GET, generateStaticParams, revalidate } =
  createDocsMarkdownRoute({
    sources: [geistdocsSource]
  });
