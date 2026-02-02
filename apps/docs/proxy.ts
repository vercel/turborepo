import { createI18nMiddleware } from "fumadocs-core/i18n/middleware";
import { isMarkdownPreferred, rewritePath } from "fumadocs-core/negotiation";
import {
  type NextFetchEvent,
  type NextRequest,
  NextResponse
} from "next/server";
import { i18n } from "@/lib/geistdocs/i18n";
import { trackMdRequest } from "@/lib/md-tracking";

const { rewrite: rewriteLLM } = rewritePath(
  "/docs{/*path}",
  "/en/llms.md{/*path}"
);

const internationalizer = createI18nMiddleware(i18n);

function trackMd(
  request: NextRequest,
  context: NextFetchEvent,
  path: string,
  requestType?: "md-url" | "header-negotiated"
): void {
  context.waitUntil(
    trackMdRequest({
      path,
      userAgent: request.headers.get("user-agent"),
      referer: request.headers.get("referer"),
      acceptHeader: request.headers.get("accept"),
      requestType
    })
  );
}

const proxy = (request: NextRequest, context: NextFetchEvent) => {
  const pathname = request.nextUrl.pathname;

  // Handle .md extension in URL path (e.g., /docs/getting-started.md or /docs.md)
  if (pathname === "/docs.md") {
    trackMd(request, context, "/llms.md");
    return NextResponse.rewrite(new URL("/en/llms.md", request.nextUrl));
  }
  if (pathname.startsWith("/docs/") && pathname.endsWith(".md")) {
    // Strip the .md extension and rewrite to llms.md route
    const pathWithoutMd = pathname.slice(0, -3); // Remove .md
    const docPath = pathWithoutMd.replace(/^\/docs\//, "");
    trackMd(request, context, `/llms.md/${docPath}`);
    return NextResponse.rewrite(
      new URL(`/en/llms.md/${docPath}`, request.nextUrl)
    );
  }

  // Handle Markdown preference via Accept header
  if (isMarkdownPreferred(request)) {
    const result = rewriteLLM(pathname);
    if (result) {
      // Track with path without lang prefix (e.g., /llms.md/getting-started)
      const trackingPath = result.replace(/^\/[a-z]{2}\//, "/");
      trackMd(request, context, trackingPath, "header-negotiated");
      return NextResponse.rewrite(new URL(result, request.nextUrl));
    }
  }

  // Fallback to i18n middleware
  return internationalizer(request, context);
};

export const config = {
  // Matcher ignoring `/_next/`, `/api/`, static assets, favicon, feed.xml, sitemap.xml, sitemap.md, robots.txt, schema JSON files, etc.
  matcher: [
    "/((?!api|_next/static|_next/image|favicon.ico|feed.xml|sitemap.xml|sitemap.md|robots.txt|schema\\.json|schema\\.v\\d+\\.json|microfrontends/schema\\.json).*)"
  ]
};

export default proxy;
