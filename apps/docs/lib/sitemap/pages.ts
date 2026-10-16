import { docs, blogDocs, openapiDocs, extraDocs } from "@/.source/server";

/**
 * A content file's path as a URL slug. A folder's `index` page is served at the
 * folder's URL, so `core-concepts/index.mdx` is `core-concepts` and a top-level
 * `index.mdx` is the empty slug.
 */
function toSlug(path: string): string {
  return path.replace(/\.mdx?$/, "").replace(/(^|\/)index$/, "");
}

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
    const slug = toSlug(doc.info.path);
    urlSet.add(slug ? `/docs/${slug}` : "/docs");
  }

  // Blog pages
  for (const doc of blogDocs) {
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    urlSet.add(`/blog/${slug}`);
  }

  // OpenAPI pages
  for (const doc of openapiDocs) {
    const slug = toSlug(doc.info.path);
    urlSet.add(slug ? `/docs/openapi/${slug}` : "/docs/openapi");
  }

  // Extra pages (governance, terms, etc.)
  for (const doc of extraDocs) {
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    urlSet.add(`/${slug}`);
  }

  return [...urlSet].sort();
}
