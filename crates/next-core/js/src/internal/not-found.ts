import { MIME_APPLICATION_JSON } from "./mime";
import type { IpcOutgoingMessage } from "./types";

export const REWRITE_NOT_FOUND = "/_next/404";

export function createNotFoundResponse(isDataReq: boolean): IpcOutgoingMessage {
  if (isDataReq) {
    return {
      type: "response",
      // Returning a 404 status code is required for the client-side router
      // to redirect to the error page.
      statusCode: 404,
      body: '{"notFound":true}',
      headers: [["Content-Type", MIME_APPLICATION_JSON]],
    };
  }

  return {
    type: "rewrite",
    // /_next/404 is a Turbopack-internal route that will always redirect to
    // the 404 page.
    path: REWRITE_NOT_FOUND,
  };
}
