import { type NextRequest, type NextFetchEvent, NextResponse } from "next/server";
import { trackMdRequest } from "@/lib/md-tracking";

export const config = {
  matcher: [
    // Match llms.txt, llms.md, and llms-full.txt for any language
    "/:lang/llms.txt",
    "/:lang/llms.md/:path*",
    "/:lang/llms-full.txt"
  ]
};

export function middleware(request: NextRequest, event: NextFetchEvent) {
  const pathname = request.nextUrl.pathname;

  // Track markdown/txt request (fire-and-forget via waitUntil)
  event.waitUntil(
    trackMdRequest({
      path: pathname,
      userAgent: request.headers.get("user-agent"),
      referer: request.headers.get("referer"),
      acceptHeader: request.headers.get("accept")
    })
  );

  return NextResponse.next();
}
