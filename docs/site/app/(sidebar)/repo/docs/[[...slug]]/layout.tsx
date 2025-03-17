import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { repoDocsPages } from "@/app/source";

export default async function SlugLayout(props: {
  params: Promise<{ slug?: string[] }>;
  children: React.ReactNode;
}): Promise<JSX.Element> {
  const params = await props.params;

  const { children } = props;

  const page = repoDocsPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  return (
    <DocsPage>
      <DocsBody>{children}</DocsBody>
    </DocsPage>
  );
}
