import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";

export function feedbackMiddleware(request: NextRequest) {
  const response = NextResponse.rewrite(
    new URL("https://vercel.com/api/feedback", request.url)
  );

  response.headers.set("Access-Control-Allow-Methods", "POST");
  return response;
}

export const feedbackSrc = "/api/feedback";
