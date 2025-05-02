import { readFileSync } from "node:fs";
import { notFound } from "next/navigation";
import type { Metadata } from "next/types";
import { repoDocsPages } from "#app/source.ts";
import { createMetadata } from "#lib/create-metadata.ts";
import { mdxComponents } from "#mdx-components.tsx";
import { CopyToMarkdown } from "#components/copy-to-markdown.tsx";
import { SystemEnvironmentVariablesHashHighlighter } from "./system-environment-variables-hash-highlighter";

export async function generateMetadata(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = repoDocsPages.getPage(params.slug);

  if (!page) notFound();

  return createMetadata({
    title: page.data.title,
    description: page.data.description,
    canonicalPath: page.url,
  });
}

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return repoDocsPages.getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export default async function Page(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = repoDocsPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  const rawMarkdown = readFileSync(page.data._file.absolutePath)
    .toString()
    // Removes frontmatter
    .replace(/^---\n(?<content>.*?\n)---\n/s, "")
    // Removes import statements for components
    .replace(
      /^import\s+{[^}]+}\s+from\s+['"]#\/[^'"]+['"];(?<lineEnding>\r?\n|$)/gm,
      ""
    );

  /* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- MDX component is dynamically imported */
  const Mdx = page.data.body;

  return (
    <>
      <SystemEnvironmentVariablesHashHighlighter />
      <div className="flex justify-between gap-4">
        <h1 className="scroll-m-7 text-4xl font-semibold tracking-normal">
          {page.data.title}
        </h1>

        <CopyToMarkdown markdownContent={rawMarkdown} />
      </div>
      <Mdx components={mdxComponents} />
    </>
  );
}
