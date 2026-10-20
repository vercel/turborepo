import { defineConfig } from "vitest/config";

export const baseConfig = defineConfig({
  test: {
    reporters: ["default", "blob"],
    outputFile: {
      blob: "coverage/blob/report.json",
    },
    coverage: {
      provider: "istanbul",
      enabled: true,
    },
  },
});
