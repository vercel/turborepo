import {
  DocsPage,
  DocsBody,
  DocsTitle,
  DocsDescription,
} from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import defaultMdxComponents from "fumadocs-ui/mdx";
import { openapi, openapiPages } from "../source";
import "./openapi.css";

export default async function Page(props: {
  params: Promise<{ slug?: Array<string> }>;
}): Promise<JSX.Element> {
  const params = await props.params;
  const page = openapiPages.getPage(params.slug);

  if (!page) {
    notFound();
  }

  /* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- MDX component is dynamically imported */
  const Mdx = page.data.body;

  return (
    <DocsPage
      /* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- Page data is dynamically generated */
      full={page.data.full}
      /* eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- Page data is dynamically generated */
      toc={page.data.toc}
    >
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- Page data is dynamically generated */}
      <DocsTitle>{page.data.title}</DocsTitle>
      {/* eslint-disable-next-line @typescript-eslint/no-unsafe-member-access -- Page data is dynamically generated */}
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <Mdx
          components={{
            ...defaultMdxComponents,
            // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- What's going on here?
            APIPage: openapi.APIPage,
          }}
        />
      </DocsBody>
    </DocsPage>
  );
}

export function generateStaticParams(): Array<{ slug: Array<string> }> {
  return openapiPages.generateParams();
}
