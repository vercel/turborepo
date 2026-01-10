const PRODUCT_DOMAIN = "turborepo.com";

/**
 * State for a single page tracked in the sitemap
 */
export interface PageState {
  /** URL path of the page */
  url: string;
  /** Hash of the meaningful content */
  contentHash: string;
  /** ISO timestamp of when content was last modified */
  lastmod: string;
  /** ISO timestamp of last crawl */
  lastCrawled: string;
}

/**
 * Overall state persisted in Redis
 */
export interface SitemapState {
  /** Version for future migration support */
  version: number;
  /** ISO timestamp of last successful crawl */
  lastCrawl: string;
  /** Map of URL path to page state */
  pages: Record<string, PageState>;
}

/**
 * Result of crawling a single page
 */
export interface CrawlResult {
  /** URL that was crawled */
  url: string;
  /** Whether the crawl succeeded */
  success: boolean;
  /** Hash of meaningful content (if successful) */
  contentHash?: string;
  /** Error message if failed */
  error?: string;
}

/**
 * Options for content extraction
 */
export interface ContentExtractionOptions {
  /** CSS selectors for main content areas (in order of preference) */
  mainContentSelectors: Array<string>;
  /** Whether to include images in content hash */
  includeImages: boolean;
  /** Whether to normalize whitespace */
  normalizeWhitespace: boolean;
  /** Whether to strip numbers */
  stripNumbers: boolean;
}

export const DEFAULT_CONTENT_OPTIONS: ContentExtractionOptions = {
  mainContentSelectors: [
    "main",
    '[role="main"]',
    "article",
    ".content",
    "#content",
    ".docs-content",
    ".markdown-body"
  ],
  includeImages: true,
  normalizeWhitespace: true,
  stripNumbers: true
};

const vercelEnv = process.env.VERCEL_ENV || "development";

export const SITEMAP_CONFIG = {
  /** Base URL for the sitemap */
  baseUrl: `https://${PRODUCT_DOMAIN}`,
  /** Request timeout in milliseconds */
  timeout: 30000,
  /** User agent string */
  userAgent: "TurborepoSitemapCrawler/1.0",
  /** Maximum concurrent requests */
  concurrency: 5,
  /** Redis key for sitemap state (namespaced by environment) */
  redisKey: `sitemap:state:${vercelEnv}`,
  /** Current state version */
  stateVersion: 1
} as const;
