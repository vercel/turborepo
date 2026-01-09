import { notFound } from "next/navigation";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle
} from "@/components/geistdocs/docs-page";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { openapi, openapiPages } from "@/lib/geistdocs/source";
import "./openapi.css";

const Page = async ({
  params
}: {
  params: Promise<{ slug?: Array<string> }>;
}) => {
  const { slug } = await params;
  const page = openapiPages.getPage(slug);

  if (!page) {
    notFound();
  }

  const MDX = page.data.body;

  return (
    <DocsPage full={page.data.full} toc={page.data.toc}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <MDX
          components={getMDXComponents({
            components: {
              APIPage: openapi.APIPage
            }
          })}
        />
      </DocsBody>
    </DocsPage>
  );
};

export const generateStaticParams = () => openapiPages.generateParams();

export default Page;
