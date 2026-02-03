import { flag } from "flags/next";
import { vercelAdapter } from "@flags-sdk/vercel";

const adapter = process.env.FLAGS ? vercelAdapter() : undefined;

export const enableDevtools = flag({
  key: "enable-devtools",
  defaultValue: process.env.NODE_ENV === "development",
  adapter
});
