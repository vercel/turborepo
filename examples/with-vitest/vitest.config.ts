import { defineConfig } from "vitest/config";
import { sharedConfig } from "@repo/vitest-config";

export default defineConfig({
  test: {
    ...sharedConfig.test,
    projects: ["apps/*/vitest.config.ts", "packages/*/vitest.config.ts"],
  },
});
