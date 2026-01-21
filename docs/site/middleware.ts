import { type NextRequest, type NextFetchEvent, NextResponse } from "next/server";
import { trackMdRequest } from "@/lib/md-tracking";

export const config = {
  matcher: [
    // Match llms.txt, llms.md, and llms-full.txt for any language
    "/:lang/llms.txt",
    "/:lang/llms.md",
    "/:lang/llms.md/:path+",
    "/:lang/llms-full.txt"
  ]
};

export function middleware(request: NextRequest, event: NextFetchEvent) {
  const pathname = request.nextUrl.pathname;

  // Strip language prefix to maintain tracking format compatibility with original route handlers
  // e.g., /en/llms.md/getting-started -> /llms.md/getting-started
  const pathWithoutLang = pathname.replace(/^\/[a-z]{2}\//, "/");

  // Track markdown/txt request (fire-and-forget via waitUntil)
  event.waitUntil(
    trackMdRequest({
      path: pathWithoutLang,
      userAgent: request.headers.get("user-agent"),
      referer: request.headers.get("referer"),
      acceptHeader: request.headers.get("accept")
    })
  );

  return NextResponse.next();
}
