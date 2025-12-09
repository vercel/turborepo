import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { repoDocsPages } from "#app/source.ts";
import { TOCSetter } from "#components/docs-layout/toc-context.tsx";

export default async function SlugLayout(props: {
  params: Promise<{ slug?: Array<string> }>;
  children: React.ReactNode;
}): Promise<JSX.Element> {
  const params = await props.params;

  const { children } = props;

  const page = repoDocsPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  return (
    <DocsPage breadcrumb={{ enabled: false }}>
      <TOCSetter toc={page.data.toc} />
      <DocsBody>{children}</DocsBody>
    </DocsPage>
  );
}
