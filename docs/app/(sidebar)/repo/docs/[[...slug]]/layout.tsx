import { SwrProvider } from "@/components/RemoteCacheCounterButRsc/swr-provider";
import {
  REMOTE_CACHE_MINUTES_SAVED_URL,
  computeTimeSaved,
  remoteCacheTimeSavedQuery,
} from "@/components/RemoteCacheCounterButRsc/data";
import { getPage, getPages } from "@/app/source";
import { DocsPage, DocsBody } from "fumadocs-ui/page";
import { notFound } from "next/navigation";
import { Suspense } from "react";
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

  // const startingAnimationNumber =
  //   computeTimeSaved(
  //     await remoteCacheTimeSavedQuery(REMOTE_CACHE_MINUTES_SAVED_URL)
  //   ) - 50;

  console.log(Date.now());

  return (
    <SwrProvider startingNumber={0}>
      <DocsPage
        toc={page.data.exports.toc}
        tableOfContent={{
          header: (
            <>
              {/* TODO: Where does the Suspense really go? */}
              <Suspense fallback={<div>Loading...</div>}>
                {/* @ts-expect-error */}
                <RemoteCacheCounterButRsc />
              </Suspense>
            </>
          ),
        }}
      >
        <DocsBody>{children}</DocsBody>
      </DocsPage>
    </SwrProvider>
  );
}

export function generateStaticParams() {
  return getPages().map((page) => ({
    slug: page.slugs,
  }));
}

export function generateMetadata({ params }: { params: { slug?: string[] } }) {
  const page = getPage(params.slug);

  if (!page) notFound();

  return {
    title: page.data.title,
    description: page.data.description,
  } satisfies Metadata;
}
