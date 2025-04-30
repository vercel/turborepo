import { notFound } from "next/navigation";
import type { Metadata } from "next";
import { extraPages } from "#app/source.ts";
import { createMetadata } from "#lib/create-metadata.ts";
import { mdxComponents } from "#mdx-components.tsx";

export default async function Page(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = extraPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  /* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- MDX component */
  const Mdx = page.data.body;

  return (
    <article className="prose pt-10 mx-auto mb-10 w-full min-w-0 max-w-5xl px-6 md:px-12">
      <h1 className="text-left">{page.data.title}</h1>
      <Mdx components={mdxComponents} />
    </article>
  );
}

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return extraPages.getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export async function generateMetadata(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = extraPages.getPage(params.slug);

  if (!page) notFound();

  return createMetadata({
    title: page.data.title,
    description: page.data.description,
    canonicalPath: params.slug?.join("/") ?? "",
  });
}
