import { docs, blogDocs, openapiDocs } from "@/.source/server";

/**
 * Collect all page URLs for the sitemap.
 *
 * Accesses fumadocs source data directly from .source/server to avoid
 * initialization issues with the loader() function in serverless environments.
 */
export function getAllPageUrls(): Array<string> {
  const urlSet = new Set<string>();

  // Static routes
  urlSet.add("/");
  urlSet.add("/blog");
  urlSet.add("/showcase");

  // Docs pages - access the docs array directly
  for (const doc of docs.docs) {
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    if (slug === "index") {
      urlSet.add("/docs");
    } else {
      urlSet.add(`/docs/${slug}`);
    }
  }

  // Blog pages
  for (const doc of blogDocs) {
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    urlSet.add(`/blog/${slug}`);
  }

  // OpenAPI pages
  for (const doc of openapiDocs) {
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    if (slug === "index") {
      urlSet.add("/docs/openapi");
    } else {
      urlSet.add(`/docs/openapi/${slug}`);
    }
  }

  return [...urlSet].sort();
}
