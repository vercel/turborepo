export const sharedConfig = {
  test: {
    globals: true,
    reporters: ["default", "blob"],
    outputFile: {
      blob: "coverage/blob/report.json",
    },
    coverage: {
      provider: "istanbul" as const,
      enabled: true,
    },
  },
};

// Re-export specific configs for backwards compatibility
export { baseConfig } from "./configs/base-config.js";
export { uiConfig } from "./configs/ui-config.js";
