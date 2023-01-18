import type { Ipc } from "@vercel/turbopack-next/ipc/index";
import type { IncomingMessage } from "node:http";
import { Buffer } from "node:buffer";
import { createServer, makeRequest } from "@vercel/turbopack-next/ipc/server";
import loadNextConfig from "@vercel/turbopack-next/entry/config/next";

import "next/dist/server/node-polyfill-fetch.js";

type RouterRequest = {
  method: string;
  pathname: string;
  // TODO: not passed to request
  headers: Record<string, string>;
  query: Record<string, string>;
};

type RouteResult = {
  url: string;
  headers: Record<string, string>;
  statusCode: number;
  isRedirect: boolean;
};

type IpcOutgoingMessage =
  | { type: "middleware-headers"; data: MiddlewareHeadersResponse }
  | { type: "middleware-body"; data: Uint8Array }
  | {
      type: "redirect";
      data: RedirectResponse;
    }
  | {
      type: "rewrite";
      data: RewriteResponse;
    };

type RedirectResponse = {
  url: string;
  statusCode: number;
  headers: string[];
};

type RewriteResponse = {
  url: string;
  statusCode: number;
  headers: string[];
};

type MiddlewareHeadersResponse = {
  statusCode: number;
  headers: string[];
};

export default async function route(
  ipc: Ipc<RouterRequest, IpcOutgoingMessage>,
  routerRequest: RouterRequest
) {
  // Deferring the import allows us to not error while we wait for Next.js to implement.
  const { makeResolver } = await import("next/dist/server/router");
  const nextConfig = loadNextConfig();

  // TODO: Need next impl. This function receives the parsed nextConfig, which it should
  // use to create a router function. The router fn will receive
  // (IncomingMessage, ServerResponse) params (which exactly match a regular
  // request/response) and returns:
  //
  // headers:
  //   'x-nextjs-route-result: 1' to signal the body has the JSON payload with result,
  //       else it streams the response headers/body as raw bytes.
  // body (if 'x-nextjs-route-result: 1'):
  // {
  //   url: '/', // resolved url (includes query info if applicable)
  //   headers: {}, // response headers to send down
  //   statusCode: 200, //
  //   isRedirect: false, //
  // }
  const resolveRoute = makeResolver(nextConfig);
  const server = await createServer();

  try {
    const {
      clientRequest,
      clientResponsePromise,
      serverRequest,
      serverResponse,
    } = await makeRequest(
      server,
      routerRequest.method,
      routerRequest.pathname,
      routerRequest.query,
      routerRequest.headers
    );

    // Send the clientRequest, so the server parses everything. We can then pass
    // the serverRequest to Next.js to handle.
    clientRequest.end();

    await Promise.all([
      resolveRoute(serverRequest, serverResponse),
      handleClientResponse(ipc, clientResponsePromise),
    ]);
    server.close();
  } catch (e) {
    ipc.sendError(e as Error);
  }
}

async function handleClientResponse(
  ipc: Ipc<RouterRequest, IpcOutgoingMessage>,
  clientResponsePromise: Promise<IncomingMessage>
) {
  const clientResponse = await clientResponsePromise;

  if (clientResponse.headers["x-nextjs-route-result"] === "1") {
    clientResponse.setEncoding("utf8");
    // We're either a redirect or a rewrite
    let buffer = "";
    for await (const chunk of clientResponse) {
      buffer += chunk;
    }

    const data = JSON.parse(buffer) as RouteResult;
    return ipc.send({
      type: data.isRedirect ? "redirect" : "rewrite",
      data: {
        url: data.url,
        statusCode: data.statusCode,
        headers: Object.entries(data.headers).flat(),
      },
    });
  }

  const responseHeaders: MiddlewareHeadersResponse = {
    statusCode: clientResponse.statusCode!,
    headers: clientResponse.rawHeaders,
  };

  ipc.send({
    type: "middleware-headers",
    data: responseHeaders,
  });

  for await (const chunk of clientResponse) {
    ipc.send({
      type: "middleware-body",
      data: chunk as Buffer,
    });
  }
}
