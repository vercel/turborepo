import { acceptsMarkdown, isAIAgent } from "@vercel/agent-readability";
import { createI18nMiddleware } from "fumadocs-core/i18n/middleware";
import {
  type NextFetchEvent,
  type NextRequest,
  NextResponse
} from "next/server";
import { i18n } from "@/lib/geistdocs/i18n";
import { trackMdRequest } from "@/lib/md-tracking";

const internationalizer = createI18nMiddleware(i18n);

function getTrackedDocsPath(pathname: string): string | null {
  if (pathname === "/docs" || pathname === "/docs.md") {
    return "/docs/index";
  }

  if (!pathname.startsWith("/docs/")) {
    return null;
  }

  const docPath = pathname.replace("/docs/", "").replace(/\.md$/, "") || "index";

  return `/docs/${docPath}`;
}

function getMarkdownRewritePath(pathname: string): string | null {
  if (pathname === "/docs" || pathname === "/docs.md") {
    return "/en/docs/md";
  }

  if (!pathname.startsWith("/docs/")) {
    return null;
  }

  const docPath = pathname.replace("/docs/", "").replace(/\.md$/, "");

  return docPath ? `/en/docs/md/${docPath}` : "/en/docs/md";
}

function trackMd(
  request: NextRequest,
  context: NextFetchEvent,
  path: string,
  requestType?: "md-url" | "header-negotiated" | "agent-rewrite"
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
    const trackingPath = getTrackedDocsPath(pathname);
    const rewritePath = getMarkdownRewritePath(pathname);

    if (trackingPath && rewritePath) {
      trackMd(request, context, trackingPath);
      return NextResponse.rewrite(new URL(rewritePath, request.nextUrl));
    }
  }

  if (pathname.startsWith("/docs/") && pathname.endsWith(".md")) {
    const trackingPath = getTrackedDocsPath(pathname);
    const rewritePath = getMarkdownRewritePath(pathname);

    if (trackingPath && rewritePath) {
      trackMd(request, context, trackingPath);
      return NextResponse.rewrite(new URL(rewritePath, request.nextUrl));
    }
  }

  // Handle Markdown preference via Accept header
  if (
    acceptsMarkdown(request) &&
    (pathname === "/docs" || pathname.startsWith("/docs/")) &&
    pathname !== "/docs/md" &&
    !pathname.startsWith("/docs/md/")
  ) {
    const trackingPath = getTrackedDocsPath(pathname);
    const rewritePath = getMarkdownRewritePath(pathname);

    if (trackingPath && rewritePath) {
      trackMd(request, context, trackingPath, "header-negotiated");
      return NextResponse.rewrite(new URL(rewritePath, request.nextUrl));
    }
  }

  // Handle AI agent detection — serve markdown automatically
  if (
    (pathname === "/docs" || pathname.startsWith("/docs/")) &&
    pathname !== "/docs/md" &&
    !pathname.startsWith("/docs/md/")
  ) {
    const { detected } = isAIAgent(request);

    if (detected) {
      const trackingPath = getTrackedDocsPath(pathname);
      const rewritePath = getMarkdownRewritePath(pathname);

      if (trackingPath && rewritePath) {
        trackMd(request, context, trackingPath, "agent-rewrite");
        return NextResponse.rewrite(new URL(rewritePath, request.nextUrl));
      }
    }
  }

  // Fallback to i18n middleware
  return internationalizer(request, context);
};

export const config = {
  matcher: [
    "/((?!api|_next/static|_next/image|favicon.ico|feed.xml|sitemap.xml|sitemap.md|robots.txt|schema\\.json|schema\\.v\\d+\\.json|microfrontends/schema\\.json).*)"
  ]
};

export default proxy;
