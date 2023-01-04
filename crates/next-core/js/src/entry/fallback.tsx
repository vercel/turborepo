import "@vercel/turbopack-next/internal/shims-client";

import { createRoot } from "react-dom/client";

import {
  initializeHMR,
  ReactDevOverlay,
} from "@vercel/turbopack-next/dev/client";
import { onUpdate } from "@vercel/turbopack-next/dev/hmr-client";

const pageChunkPath = location.pathname.slice(1);

// We don't need a full `initialize()` here as the page will be reloaded on
// update anyway. Including `next/dist/client` in this chunk causes a 70ms
// slowdown on startup.
const nextData = JSON.parse(
  document.getElementById("__NEXT_DATA__")!.textContent!
);
const assetPrefix: string = nextData.assetPrefix || "";

onUpdate(
  {
    path: pageChunkPath,
    headers: {
      accept: "text/html",
    },
  },
  (update) => {
    if (update.type === "restart") {
      location.reload();
    }
  }
);

initializeHMR({
  assetPrefix,
});

const el = document.getElementById("__next")!;
el.innerText = "";

createRoot(el).render(<ReactDevOverlay />);
