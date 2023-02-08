// middleware.ts
import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

export function middleware(request: NextRequest) {
  return NextResponse.redirect(new URL("/about-2", request.url));
}

export const config = {
  matcher: "/about/:path*",
};
