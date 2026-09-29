import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { createMetadata } from "@/lib/create-metadata";
import { getLocalizedPath } from "@/lib/geistdocs/public-path";
import { extraPages } from "@/lib/geistdocs/source";

export const createExtraPage = (slug: string) => {
  const getPage = () => extraPages.getPage([slug]) ?? notFound();

  const Page = () => {
    const page = getPage();
    const MDX = page.data.body;

    return (
      <main className="prose mx-auto mb-10 w-full min-w-0 max-w-5xl px-6 pt-10 md:px-12">
        <h1 className="text-left">{page.data.title}</h1>
        <MDX components={getMDXComponents()} />
      </main>
    );
  };

  const generateMetadata = (lang: string): Metadata => {
    const page = getPage();

    return createMetadata({
      title: page.data.title,
      description: page.data.description,
      canonicalPath: getLocalizedPath(lang, `/${slug}`)
    });
  };

  return { Page, generateMetadata };
};
