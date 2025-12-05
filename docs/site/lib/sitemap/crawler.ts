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
  const fullUrl = `${SITEMAP_CONFIG.baseUrl}${
    url.startsWith("/") ? url : `/${url}`
  }`;

  // Validate host to prevent SSRF attacks
  const parsedUrl = new URL(fullUrl);
  const allowedHost = new URL(SITEMAP_CONFIG.baseUrl).host;
  if (parsedUrl.host !== allowedHost) {
    return { url, success: false, error: "Invalid host" };
  }

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
 * Creates a concurrency limiter that allows up to `limit` concurrent executions
 */
function createLimiter(limit: number) {
  let activeCount = 0;
  const queue: Array<() => void> = [];

  const next = () => {
    if (queue.length > 0 && activeCount < limit) {
      activeCount++;
      const resolve = queue.shift();
      if (resolve) {
        resolve();
      }
    }
  };

  return async <T>(fn: () => Promise<T>): Promise<T> => {
    await new Promise<void>((resolve) => {
      queue.push(resolve);
      next();
    });

    try {
      return await fn();
    } finally {
      activeCount--;
      next();
    }
  };
}

/**
 * Crawl multiple pages with concurrency limit
 * Uses a semaphore-based pattern to maximize throughput - slow requests don't block slots
 */
export async function crawlPages(
  urls: Array<string>,
  onProgress?: (completed: number, total: number) => void
): Promise<Map<string, CrawlResult>> {
  const results = new Map<string, CrawlResult>();
  const limit = createLimiter(SITEMAP_CONFIG.concurrency);
  let completed = 0;

  const crawlResults = await Promise.all(
    urls.map((url) =>
      limit(async () => {
        const result = await crawlPage(url);
        completed++;
        onProgress?.(completed, urls.length);
        return { url, result };
      })
    )
  );

  for (const { url, result } of crawlResults) {
    results.set(url, result);
  }

  return results;
}
