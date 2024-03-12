import { getPage } from "@/app/source";
import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { RemoteCacheCounter } from "@/components/RemoteCacheCounter";

export default async function SlugLayout({
  children,
  params,
}: {
  params: { slug?: string[] };
  children: React.ReactNode;
}) {
  const page = getPage(params.slug);

  if (!page) {
    notFound();
  }

  return (
    <DocsPage
      toc={page.data.exports.toc}
      tableOfContent={{
        header: <RemoteCacheCounter />,
      }}
    >
      <DocsBody>{children}</DocsBody>
    </DocsPage>
  );
}
