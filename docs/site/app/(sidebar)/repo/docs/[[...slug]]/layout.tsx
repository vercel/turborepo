import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { FeedbackWidget } from "@/components/feedback-widget";
import { repoDocsPages } from "@/app/source";
import { RemoteCacheCounter } from "@/components/remote-cache-counter";

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
    <DocsPage
      tableOfContent={{
        header: (
          <>
            <RemoteCacheCounter />
            <FeedbackWidget />
          </>
        ),
      }}
      toc={page.data.toc}
    >
      <DocsBody>{children}</DocsBody>
    </DocsPage>
  );
}
