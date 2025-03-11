// vite.config.ts
import { defineConfig } from "vite";
import path, { resolve } from "path";
import solidPlugin from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [tailwindcss(), solidPlugin()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
      "@Components": path.resolve(__dirname, "src/components"),
      "@Configs": path.resolve(__dirname, "src/config"),
      "@Utils": path.resolve(__dirname, "src/utils"),
    },
  },
});
