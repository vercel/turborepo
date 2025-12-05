import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/**
 * Discovers all page routes from the Next.js app directory.
 *
 * This scans the app directory for all page.tsx files and converts
 * them to URL paths, ensuring the sitemap is always exhaustive.
 *
 * Route conventions handled:
 * - (group) folders - stripped from URL path
 * - [[...slug]] - catch-all routes (handled by fumadocs)
 * - [...slug] - catch-all routes (handled by fumadocs)
 * - [param] - dynamic routes (skipped, handled by fumadocs)
 * - page.tsx at root - becomes "/"
 */

/** Routes that should be excluded from the sitemap */
const EXCLUDED_ROUTES = new Set([
  "/confirm", // Thank you page, not meant for SEO indexing
]);

/**
 * Check if a route segment is a route group (parentheses)
 */
function isRouteGroup(segment: string): boolean {
  return segment.startsWith("(") && segment.endsWith(")");
}

/**
 * Check if a route segment is dynamic (brackets)
 */
function isDynamicSegment(segment: string): boolean {
  return segment.startsWith("[") && segment.endsWith("]");
}

/**
 * Recursively find all page.tsx files in a directory
 */
function findPageFiles(dir: string, basePath: string = ""): Array<string> {
  const pages: Array<string> = [];

  let entries: Array<string>;
  try {
    entries = readdirSync(dir);
  } catch {
    return pages;
  }

  for (const entry of entries) {
    const fullPath = join(dir, entry);

    let stat;
    try {
      stat = statSync(fullPath);
    } catch {
      continue;
    }

    if (stat.isDirectory()) {
      // Skip node_modules and hidden directories
      if (entry.startsWith(".") || entry === "node_modules") {
        continue;
      }

      // Skip api routes - they're not pages
      if (entry === "api") {
        continue;
      }

      // Build the URL path segment
      let urlSegment: string;
      if (isRouteGroup(entry)) {
        // Route groups don't affect the URL
        urlSegment = "";
      } else if (isDynamicSegment(entry)) {
        // Dynamic segments are handled by fumadocs loaders
        // We still recurse to find static pages within
        urlSegment = entry;
      } else {
        urlSegment = entry;
      }

      const newBasePath = urlSegment
        ? basePath
          ? `${basePath}/${urlSegment}`
          : urlSegment
        : basePath;

      pages.push(...findPageFiles(fullPath, newBasePath));
    } else if (entry === "page.tsx" || entry === "page.ts") {
      // Found a page file
      // Skip if the path contains dynamic segments (handled by fumadocs)
      if (!basePath.includes("[")) {
        const urlPath = basePath ? `/${basePath}` : "/";
        pages.push(urlPath);
      }
    }
  }

  return pages;
}

/**
 * Convert app directory path to URL path
 */
export function discoverStaticRoutes(appDir: string): Array<string> {
  const routes = findPageFiles(appDir);

  // Filter out excluded routes and deduplicate
  const uniqueRoutes = [...new Set(routes)].filter(
    (route) => !EXCLUDED_ROUTES.has(route)
  );

  return uniqueRoutes.sort();
}

/**
 * Get the app directory path relative to the current working directory
 */
export function getAppDirectory(): string {
  // In Next.js, the app directory is at the root of the project
  // This function returns the path that should work both in development
  // and when the code is run from the project root
  return join(process.cwd(), "app");
}
