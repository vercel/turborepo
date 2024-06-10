import { connect } from "@vercel/turbopack-ecmascript-runtime/browser/client/hmr-client";
import {
  connectHMR,
  addMessageListener,
  sendMessage,
} from "@vercel/turbopack-ecmascript-runtime/browser/client/websocket";

export function initializeHMR(options: { assetPrefix: string }) {
  connect({
    addMessageListener,
    sendMessage,
    onUpdateError: console.error,
  });
  connectHMR({
    assetPrefix: options.assetPrefix,
    log: true,
    path: "/turbopack-hmr",
  });
}
