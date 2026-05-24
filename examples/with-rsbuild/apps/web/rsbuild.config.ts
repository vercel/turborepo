import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

export default defineConfig({
  html: {
    favicon: "./public/rsbuild-favicon.png",
  },
  plugins: [pluginReact()],
});
