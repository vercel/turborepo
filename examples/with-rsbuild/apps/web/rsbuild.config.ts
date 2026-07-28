import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

export default defineConfig({
  html: {
    favicon: "./public/favicon.ico",
  },
  plugins: [pluginReact()],
});
