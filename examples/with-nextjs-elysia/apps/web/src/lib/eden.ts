import { treaty } from "@elysiajs/eden";
import type { App } from "~/app";
import { app } from "~/app";

// On the server (and during the build) call the Elysia app directly without
// a network round trip. In the browser, send requests to the Next.js server.
export const api =
  typeof process === "undefined"
    ? treaty<App>("localhost:3001").api.v1
    : treaty<App>(app).api.v1;
