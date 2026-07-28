import { timingSafeEqual } from "node:crypto";
import { NextResponse } from "next/server";
import { crawlPages } from "@/lib/sitemap/crawler";
import { getAllPageUrls } from "@/lib/sitemap/pages";
import {
  createEmptyState,
  loadState,
  pruneRemovedPages,
  saveState,
  updatePageState
} from "@/lib/sitemap/redis";
import { SITEMAP_CONFIG } from "@/lib/sitemap/types";

export const dynamic = "force-dynamic";
export const maxDuration = 300; // 5 minutes max for crawling

function isValidCronRequest(request: Request, cronSecret: string): boolean {
  const authHeader = request.headers.get("authorization");
  const token = authHeader?.startsWith("Bearer ")
    ? authHeader.slice("Bearer ".length)
    : null;

  if (!token) {
    return false;
  }

  const tokenBuffer = Buffer.from(token);
  const secretBuffer = Buffer.from(cronSecret);

  if (tokenBuffer.length !== secretBuffer.length) {
    return false;
  }

  return timingSafeEqual(tokenBuffer, secretBuffer);
}

/**
 * GET handler for Vercel Cron
 * Crawls all pages and updates sitemap state in Redis
 */
export async function GET(request: Request): Promise<Response> {
  const cronSecret = process.env.CRON_SECRET;

  if (!cronSecret) {
    console.error("CRON_SECRET is not configured");
    return new NextResponse("Internal Server Error", { status: 500 });
  }

  if (!isValidCronRequest(request, cronSecret)) {
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
      baseUrl: SITEMAP_CONFIG.baseUrl
    };

    // eslint-disable-next-line no-console -- Intentional logging for cron job monitoring
    console.log("Sitemap crawl completed:", summary);

    return NextResponse.json(summary);
  } catch (error) {
    // eslint-disable-next-line no-console -- Intentional logging for error tracking
    console.error("Sitemap crawl failed:", error);

    return NextResponse.json(
      {
        success: false
      },
      { status: 500 }
    );
  }
}
