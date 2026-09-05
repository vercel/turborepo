import { notFound } from "next/navigation";
import { cacheLife } from "next/cache";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle
} from "@/components/geistdocs/docs-page";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { APIPage } from "@/components/api-page";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";
import { openapiPages } from "@/lib/geistdocs/source";
import "./openapi.css";

const CachedPage = async ({ slug }: { slug?: string[] }) => {
  "use cache";
  cacheLife("max");

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
              APIPage
            }
          })}
        />
      </DocsBody>
    </DocsPage>
  );
};

const Page = async ({
  params
}: PageProps<"/[lang]/docs/openapi/[[...slug]]">) => {
  const { slug } = await params;

  return <CachedPage slug={slug} />;
};

export const generateStaticParams = () => openapiPages.generateParams();

export const generateMetadata = async ({
  params
}: PageProps<"/[lang]/docs/openapi/[[...slug]]">) => {
  const { lang, slug } = await params;
  const page = openapiPages.getPage(slug);

  if (!page) {
    notFound();
  }

  return createMetadata({
    title: page.data.title,
    description: page.data.description,
    canonicalPath: getLocalizedPath(
      lang,
      `/docs/openapi${slug?.length ? `/${slug.join("/")}` : ""}`
    )
  });
};

export default Page;
