import { source } from "@/lib/geistdocs/source";
import { blogDocs, openapiDocs } from "@/.source/server";

/**
 * Collect all page URLs for the sitemap.
 *
 * Uses fumadocs source data directly to avoid initialization issues
 * with the loader() function in serverless environments.
 */
export function getAllPageUrls(): Array<string> {
  const urlSet = new Set<string>();

  // Static routes
  urlSet.add("/");
  urlSet.add("/blog");
  urlSet.add("/showcase");

  // Docs pages - use "en" locale since source has i18n enabled
  for (const page of source.getPages("en")) {
    urlSet.add(page.url);
  }

  // Blog pages - access docs directly instead of through loader
  for (const doc of blogDocs) {
    // Build URL from slug: content/blog/foo.mdx -> /blog/foo
    const slug = doc.info.path.replace(/\.mdx?$/, "");
    urlSet.add(`/blog/${slug}`);
  }

  // OpenAPI pages - access docs directly instead of through loader
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
