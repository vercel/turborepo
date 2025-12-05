import { repoDocsPages, blog, extraPages } from "#app/source.ts";
import { openapiPages } from "#app/(openapi)/docs/openapi/source.ts";

/**
 * Collect all page URLs from fumadocs loaders
 */
export function getAllPageUrls(): Array<string> {
  const urls: Array<string> = [];

  // Add homepage
  urls.push("/");

  // Docs pages
  for (const page of repoDocsPages.getPages()) {
    urls.push(page.url);
  }

  // Blog pages (exclude external blog posts)
  for (const page of blog.getPages()) {
    urls.push(page.url);
  }

  // Extra pages (governance, terms, etc.)
  for (const page of extraPages.getPages()) {
    urls.push(page.url);
  }

  // OpenAPI pages
  for (const page of openapiPages.getPages()) {
    urls.push(page.url);
  }

  // Add showcase page
  urls.push("/showcase");

  return urls;
}
