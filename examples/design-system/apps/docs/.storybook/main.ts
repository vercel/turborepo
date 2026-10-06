import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: ["../stories/**/*.stories.tsx", "../stories/**/*.mdx"],
  addons: ["@storybook/addon-links", "@storybook/addon-docs"],
  framework: "@storybook/react-vite",
};

export default config;
