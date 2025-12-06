import { NextResponse } from "next/server";
import {
  loadState,
  saveState,
  createEmptyState,
  updatePageState,
  pruneRemovedPages,
  crawlPages,
  getAllPageUrls,
  SITEMAP_CONFIG,
} from "#lib/sitemap/index.ts";

export const dynamic = "force-dynamic";
export const maxDuration = 300; // 5 minutes max for crawling

/**
 * GET handler for Vercel Cron
 * Crawls all pages and updates sitemap state in Redis
 */
export async function GET(request: Request): Promise<Response> {
  // Verify cron secret
  const authHeader = request.headers.get("authorization");
  const cronSecret = process.env.CRON_SECRET;

  if (!cronSecret && process.env.NODE_ENV === "production") {
    return new NextResponse("Server configuration error", { status: 500 });
  }

  if (authHeader !== `Bearer ${cronSecret}`) {
    return new NextResponse("Unauthorized", { status: 401 });
  }

  const startTime = Date.now();

  try {
    // Load existing state
    let state = await loadState();
    const isNewState = !state;

    if (!state) {
      state = createEmptyState();
    }

    // Get all page URLs from fumadocs
    const pageUrls = getAllPageUrls();
    const urlSet = new Set(pageUrls);

    // Crawl all pages
    let newPages = 0;
    let updatedPages = 0;
    let unchangedPages = 0;
    let failedPages = 0;
    const errors: Array<{ url: string; error: string }> = [];

    const results = await crawlPages(pageUrls);

    // Process results
    for (const [url, result] of results) {
      if (!result.success) {
        failedPages++;
        errors.push({ url, error: result.error ?? "Unknown error" });
        continue;
      }

      if (!result.contentHash) {
        failedPages++;
        errors.push({ url, error: "No content hash returned" });
        continue;
      }

      const { updated, isNew } = updatePageState(
        state,
        url,
        result.contentHash
      );

      if (isNew) {
        newPages++;
      } else if (updated) {
        updatedPages++;
      } else {
        unchangedPages++;
      }
    }

    // Prune pages that no longer exist
    const removed = pruneRemovedPages(state, urlSet);

    // Update state timestamp
    state.lastCrawl = new Date().toISOString();

    // Save state
    await saveState(state);

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);

    const summary = {
      success: true,
      isNewState,
      elapsed: `${elapsed}s`,
      totalPages: pageUrls.length,
      newPages,
      updatedPages,
      unchangedPages,
      failedPages,
      removedPages: removed.length,
      errors: errors.slice(0, 10), // Limit errors in response
      baseUrl: SITEMAP_CONFIG.baseUrl,
    };

    // eslint-disable-next-line no-console -- Intentional logging for cron job monitoring
    console.log("Sitemap crawl completed:", summary);

    return NextResponse.json(summary);
  } catch (error) {
    // eslint-disable-next-line no-console -- Intentional logging for error tracking
    console.error("Sitemap crawl failed:", error);

    return NextResponse.json(
      {
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      { status: 500 }
    );
  }
}
