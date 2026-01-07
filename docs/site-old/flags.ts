import { flag } from "flags/next";
import { vercelAdapter } from "@flags-sdk/vercel";

export const enableDevtools = flag({
  key: "enable-devtools",
  adapter: vercelAdapter()
});
