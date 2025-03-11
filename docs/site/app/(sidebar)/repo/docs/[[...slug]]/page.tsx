import { notFound } from "next/navigation";
import type { Metadata } from "next/types";
import { repoDocsPages } from "@/app/source";
import { createMetadata } from "@/lib/create-metadata";
import { mdxComponents } from "@/mdx-components";
import { SystemEnvironmentVariablesHashHighlighter } from "./system-environment-variables-hash-highlighter";

export async function generateMetadata(props: {
  params: Promise<{ slug?: string[] }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = repoDocsPages.getPage(params.slug);

  if (!page) notFound();

  return createMetadata({
    title: page.data.title,
    product: "repo",
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

  const Mdx = page.data.body;

  return (
    <>
      <SystemEnvironmentVariablesHashHighlighter />
      <h1 className="text-left">{page.data.title}</h1>
      <Mdx components={mdxComponents} />
    </>
  );
}
