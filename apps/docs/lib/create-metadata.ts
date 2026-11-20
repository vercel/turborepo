import type { Metadata } from "next/types";
import { siteUrl } from "@/lib/geistdocs/site-url";
import { createSignedOgUrl } from "@/lib/og/sign";

/**
 * Creates a signed OG image URL for the given title.
 * For index pages (home, /repo), the title is omitted.
 */
const createOgImagePath = ({
  title,
  canonicalPath
}: {
  title?: string;
  canonicalPath: string;
}): string => {
  const isIndex = canonicalPath === "" || canonicalPath === "/";
  const isRepoIndex = canonicalPath === "/repo";

  // For index pages, use an empty title.
  const ogTitle = isIndex || isRepoIndex ? "" : title || "";

  return createSignedOgUrl(
    ogTitle,
    canonicalPath.startsWith("/blog/") ? "Blog" : undefined
  );
};

/**
 * A standardized, utility-ized replacement for generateMetadata.
 * Creates metadata with signed OG image URLs.
 */
export const createMetadata = ({
  title,
  description,
  canonicalPath
}: {
  title?: string;
  description?: string;
  /** You do not need to supply the domain! `metadataBase` already does that for you. */
  canonicalPath: string;
}): Metadata => {
  if (!description) {
    // eslint-disable-next-line no-console -- We want to be alerted during a build if this happens
    console.warn(`Warning: ${canonicalPath} does not have a description.`);
  }

  return {
    metadataBase: siteUrl,
    title: title ? `${title} | Turborepo` : "Turborepo",
    description,
    openGraph: {
      siteName: "Turborepo",
      images: [
        createOgImagePath({
          title: canonicalPath === "/" ? "" : title,
          canonicalPath
        })
      ],
      url: canonicalPath
    },
    alternates: {
      canonical: canonicalPath
    }
  };
};
