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

  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Not typed properly?
  const Mdx = page.data.body;

  return (
    <DocsPage full={page.data.full} toc={page.data.toc}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <Mdx
          components={{
            ...defaultMdxComponents,
            // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Not typed properly?
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
