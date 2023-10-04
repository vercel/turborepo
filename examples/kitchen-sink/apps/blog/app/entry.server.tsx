import handleRequest from "@vercel/remix-entry-server";
import { RemixServer } from "@remix-run/react";
import type { EntryContext } from "@remix-run/server-runtime";

export default function entry(
  request: Request,
  responseStatusCode: number,
  responseHeaders: Headers,
  remixContext: EntryContext
): Promise<unknown> {
  const remixServer = <RemixServer context={remixContext} url={request.url} />;
  return handleRequest(
    request,
    responseStatusCode,
    responseHeaders,
    remixServer
  );
}
