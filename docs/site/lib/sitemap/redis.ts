import { Redis } from "@upstash/redis";
import type { SitemapState, PageState } from "./types";
import { SITEMAP_CONFIG } from "./types";

// Initialize Redis from environment variables
const redis = Redis.fromEnv();

/**
 * Load sitemap state from Redis
 */
export async function loadState(): Promise<SitemapState | null> {
  try {
    const state = await redis.get<SitemapState>(SITEMAP_CONFIG.redisKey);

    if (!state) {
      return null;
    }

    // Validate version
    if (state.version !== SITEMAP_CONFIG.stateVersion) {
      // eslint-disable-next-line no-console -- Intentional logging for debugging
      console.warn(
        `State version mismatch. Expected ${SITEMAP_CONFIG.stateVersion}, got ${state.version}. Starting fresh.`
      );
      return null;
    }

    return state;
  } catch (error) {
    // eslint-disable-next-line no-console -- Intentional logging for debugging
    console.error("Failed to load state from Redis:", error);
    return null;
  }
}

/**
 * Save sitemap state to Redis
 */
export async function saveState(state: SitemapState): Promise<void> {
  await redis.set(SITEMAP_CONFIG.redisKey, state);
}

/**
 * Create a new empty state
 */
export function createEmptyState(): SitemapState {
  return {
    version: SITEMAP_CONFIG.stateVersion,
    lastCrawl: new Date().toISOString(),
    pages: {},
  };
}

/**
 * Update state with a crawl result for a single page
 * Returns whether the content changed (lastmod should be updated)
 */
export function updatePageState(
  state: SitemapState,
  url: string,
  contentHash: string
): { updated: boolean; isNew: boolean } {
  const now = new Date().toISOString();
  const existingPage = state.pages[url] as PageState | undefined;

  if (existingPage !== undefined) {
    // Existing page - check if content changed
    const contentChanged = existingPage.contentHash !== contentHash;

    // Update the page state
    existingPage.contentHash = contentHash;
    existingPage.lastCrawled = now;

    if (contentChanged) {
      existingPage.lastmod = now;
    }

    return { updated: contentChanged, isNew: false };
  }

  // New page
  state.pages[url] = {
    url,
    contentHash,
    lastmod: now,
    lastCrawled: now,
  };
  return { updated: true, isNew: true };
}

/**
 * Remove pages that no longer exist (weren't in the URL list)
 */
export function pruneRemovedPages(
  state: SitemapState,
  currentUrls: Set<string>
): Array<string> {
  const removed: Array<string> = [];
  const urlsToRemove: Array<string> = [];

  for (const url of Object.keys(state.pages)) {
    if (!currentUrls.has(url)) {
      urlsToRemove.push(url);
    }
  }

  for (const url of urlsToRemove) {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- Needed for pruning pages
    delete state.pages[url];
    removed.push(url);
  }

  return removed;
}

/**
 * Get all page states sorted by URL
 */
export function getPagesSorted(state: SitemapState): Array<PageState> {
  return Object.values(state.pages).sort((a, b) => a.url.localeCompare(b.url));
}
