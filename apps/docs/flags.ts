import { flag } from "flags/next";
import { vercelAdapter } from "@flags-sdk/vercel";

function getVercelAdapter() {
  const flags = process.env.FLAGS;

  if (!flags?.startsWith("vf_") && !flags?.startsWith("flags:")) {
    return undefined;
  }

  return vercelAdapter();
}

const adapter = getVercelAdapter();

export const enableDevtools = flag({
  key: "enable-devtools",
  defaultValue: process.env.NODE_ENV === "development",
  adapter,
  decide() {
    return process.env.NODE_ENV === "development";
  }
});
