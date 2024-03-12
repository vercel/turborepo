import { getPage, getPages } from "@/app/source";
import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { Metadata } from "next";
import { RemoteCacheCounterButRsc } from "@/components/RemoteCacheCounterButRsc";

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
        header: <RemoteCacheCounterButRsc />,
      }}
    >
      <DocsBody>{children}</DocsBody>
    </DocsPage>
  );
}
