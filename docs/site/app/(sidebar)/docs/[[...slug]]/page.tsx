import { notFound } from "next/navigation";
import { readFileSync } from "fs";
import type { Metadata } from "next/types";
import { repoDocsPages } from "@/app/source";
import { createMetadata } from "@/lib/create-metadata";
import { mdxComponents } from "@/mdx-components";
import { SystemEnvironmentVariablesHashHighlighter } from "./system-environment-variables-hash-highlighter";
import { CopyToMarkdown } from "@/components/copy-to-markdown";

export async function generateMetadata(props: {
  params: Promise<{ slug?: string[] }>;
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

export function generateStaticParams(): { slug: string[] }[] {
  return repoDocsPages.getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export default async function Page(props: {
  params: Promise<{ slug?: string[] }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = repoDocsPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  const rawMarkdown = readFileSync(page.data._file.absolutePath)
    .toString()
    // TODO: Make the regex capture all of the header metadata
    .replace(/---/g, "")
    // TODO: Regex to capture all import lines for real
    .replace(/import { Step, Steps } from \'#\/components\/steps\';/g, "")
    .replace(/import { Tabs, Tab } from \'#\/components\/tabs\';/g, "")
    .replace(/import { Callout } from \'#\/components\/callout\';/g, "");

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
