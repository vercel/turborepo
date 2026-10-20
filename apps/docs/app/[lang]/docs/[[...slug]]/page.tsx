import { MobileDocsBar } from "@vercel/geistdocs/mobile-docs-bar";
import {
  createDocsPage,
  createPageActions
} from "@vercel/geistdocs/pages/docs";
import type { MDXComponents } from "mdx/types";
import { getMDXComponents } from "@/components/geistdocs/mdx-components";
import { RemoteCacheCounter } from "@/components/remote-cache-counter";
import { config } from "@/lib/geistdocs/config";
import { geistdocsSource, getPageImage } from "@/lib/geistdocs/source";

const docsPage = createDocsPage({
  config,
  mdx: ({ link }: { link: MDXComponents["a"] }) =>
    getMDXComponents({ components: { a: link } }),
  metadata: ({ metadata, page }) => ({
    ...metadata,
    openGraph: {
      ...metadata.openGraph,
      // Keep the site's HMAC-signed OG image URLs instead of the package's
      // unsigned /og/... URLs (the OG route verifies signatures).
      images: getPageImage(page).url
    }
  }),
  pageActions: createPageActions({
    config,
    getExtraActions: () => [<RemoteCacheCounter key="remote-cache-counter" />]
  }),
  renderTop: ({ data }) => <MobileDocsBar toc={data.toc} />,
  source: geistdocsSource,
  tableOfContentPopover: {
    enabled: false
  }
});

export default docsPage.Page;
export const generateStaticParams = docsPage.generateStaticParams;
export const generateMetadata = docsPage.generateMetadata;
