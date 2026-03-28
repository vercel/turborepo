import type { Metadata } from "next/types";
import { createSignedOgUrl } from "@/lib/og/sign";

const getBaseURL = (): URL => {
  if (process.env.VERCEL_ENV === "production") {
    return new URL(`https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`);
  }

  if (process.env.VERCEL_ENV === "preview") {
    return new URL(`https://${process.env.VERCEL_URL}`);
  }

  return new URL(`http://localhost:${process.env.PORT || 3000}`);
};

/**
 * Creates a signed OG image URL for the given title.
 * For index pages (home, /repo), the title is omitted to show just the logo.
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

  // For index pages, use empty title (logo only)
  const ogTitle = isIndex || isRepoIndex ? "" : title || "";

  return createSignedOgUrl(ogTitle);
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
    metadataBase: getBaseURL(),
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
