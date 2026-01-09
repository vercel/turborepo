import { type VercelConfig } from "@vercel/config/v1";

export const config: VercelConfig = {
  buildCommand: "turbo build && cd content/openapi/artifacts && ls -la"
};
