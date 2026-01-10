import { createI18nMiddleware } from "fumadocs-core/i18n/middleware";
import { isMarkdownPreferred, rewritePath } from "fumadocs-core/negotiation";
import {
  type NextFetchEvent,
  type NextRequest,
  NextResponse
} from "next/server";
import { i18n } from "@/lib/geistdocs/i18n";

const { rewrite: rewriteLLM } = rewritePath("/docs/*path", "/llms.md/*path");

const internationalizer = createI18nMiddleware(i18n);

const proxy = (request: NextRequest, context: NextFetchEvent) => {
  const pathname = request.nextUrl.pathname;

  // OpenAPI pages should not be proxied
  if (pathname.startsWith("/docs/openapi")) {
    return NextResponse.next();
  }

  // Handle .md extension in URL path (e.g., /docs/getting-started.md or /docs.md)
  if (pathname === "/docs.md") {
    return NextResponse.rewrite(new URL("/en/llms.md", request.nextUrl));
  }
  if (pathname.startsWith("/docs/") && pathname.endsWith(".md")) {
    // Strip the .md extension and rewrite to llms.md route
    const pathWithoutMd = pathname.slice(0, -3); // Remove .md
    const docPath = pathWithoutMd.replace(/^\/docs\//, "");
    return NextResponse.rewrite(
      new URL(`/en/llms.md/${docPath}`, request.nextUrl)
    );
  }

  // Handle Markdown preference via Accept header
  if (isMarkdownPreferred(request)) {
    const result = rewriteLLM(pathname);
    if (result) {
      return NextResponse.rewrite(new URL(result, request.nextUrl));
    }
  }

  // Fallback to i18n middleware
  return internationalizer(request, context);
};

export const config = {
  // Matcher ignoring `/_next/`, `/api/`, static assets, favicon, feed.xml, etc.
  matcher: ["/((?!api|_next/static|_next/image|favicon.ico|feed.xml).*)"]
};

export default proxy;
