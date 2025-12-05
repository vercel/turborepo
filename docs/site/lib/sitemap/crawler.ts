import type { CrawlResult, ContentExtractionOptions } from "./types";
import { DEFAULT_CONTENT_OPTIONS, SITEMAP_CONFIG } from "./types";
import { extractContentHash } from "./content-extractor";

/**
 * Crawl a single URL and extract content hash
 */
export async function crawlPage(
  url: string,
  contentOptions: ContentExtractionOptions = DEFAULT_CONTENT_OPTIONS
): Promise<CrawlResult> {
  const fullUrl = url.startsWith("http")
    ? url
    : `${SITEMAP_CONFIG.baseUrl}${url}`;

  try {
    const response = await fetch(fullUrl, {
      headers: {
        "User-Agent": SITEMAP_CONFIG.userAgent,
      },
      signal: AbortSignal.timeout(SITEMAP_CONFIG.timeout),
    });

    if (!response.ok) {
      return {
        url,
        success: false,
        error: `HTTP ${response.status}: ${response.statusText}`,
      };
    }

    // Only process HTML pages
    const contentType = response.headers.get("content-type") || "";
    if (!contentType.includes("text/html")) {
      return {
        url,
        success: false,
        error: `Not HTML: ${contentType}`,
      };
    }

    const html = await response.text();
    const contentHash = extractContentHash(html, contentOptions);

    return {
      url,
      success: true,
      contentHash,
    };
  } catch (error) {
    return {
      url,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

/**
 * Crawl multiple pages with concurrency limit
 */
export async function crawlPages(
  urls: Array<string>,
  onProgress?: (completed: number, total: number) => void
): Promise<Map<string, CrawlResult>> {
  const results = new Map<string, CrawlResult>();
  const { concurrency } = SITEMAP_CONFIG;

  // Process in batches using Promise.all for each batch
  const batches: Array<Array<string>> = [];
  for (let i = 0; i < urls.length; i += concurrency) {
    batches.push(urls.slice(i, i + concurrency));
  }

  for (const [batchIndex, batch] of batches.entries()) {
    // eslint-disable-next-line no-await-in-loop -- Intentional batching for rate limiting
    const batchResults = await Promise.all(
      batch.map(async (url) => {
        const result = await crawlPage(url);
        return { url, result };
      })
    );

    for (const { url, result } of batchResults) {
      results.set(url, result);
    }

    const completed = Math.min((batchIndex + 1) * concurrency, urls.length);
    onProgress?.(completed, urls.length);
  }

  return results;
}
