import type { Metadata } from "next/types";

const getBaseURL = (): URL => {
  if (process.env.VERCEL_ENV === "production") {
    return new URL(`https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`);
  }

  if (process.env.VERCEL_ENV === "preview") {
    return new URL(`https://${process.env.VERCEL_URL}`);
  }

  return new URL(`http://localhost:${process.env.PORT || 3335}`);
};

const createOgImagePath = ({
  title,
  product,
  canonicalPath,
}: {
  title?: string;
  product?: string;
  canonicalPath: string;
}): URL => {
  const ogURL = new URL(`/api/og`, getBaseURL());

  if (title) {
    ogURL.searchParams.set("title", title);
  }

  if (product) {
    ogURL.searchParams.set("type", product);
  }

  const isIndex = canonicalPath === "";
  const isRepoIndex = canonicalPath === "/repo";

  if (isIndex || isRepoIndex) {
    ogURL.searchParams.delete("title");
  }

  return ogURL;
};

/** A standardized, utility-ized replacement for generateMetadata. If you need async, see `asyncCreateMetadata`. */
export const createMetadata = ({
  title,
  description,
  canonicalPath,
  product,
}: {
  product?: Products;
  title?: string;
  description?: string;
  /** You do not need to supply the domain! `metadataBase` already does that for you. */
  canonicalPath: string;
}): Metadata => {
  if (!description) {
    // eslint-disable-next-line no-console
    console.warn(`Warning: ${canonicalPath} does not have a description.`);
  }

  const formatTitle = (): string => {
    if (canonicalPath === "/repo") {
      return "Turborepo";
    }

    if (product === "repo") {
      return `${title} | Turborepo`;
    }

    return title ?? "Turbo";
  };

  const formattedTitle = formatTitle();

  return {
    metadataBase: getBaseURL(),
    title: formattedTitle,
    description,
    openGraph: {
      siteName: "Turbo",
      images: [
        createOgImagePath({
          title: canonicalPath === "/" ? "" : title,
          product,
          canonicalPath,
        }),
      ],
      url: canonicalPath,
    },
    alternates: {
      canonical: canonicalPath,
    },
  };
};
