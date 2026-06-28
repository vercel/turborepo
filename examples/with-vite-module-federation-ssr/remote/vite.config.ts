import { federation } from "@module-federation/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    federation({
      dts: false,
      name: "remote",
      filename: "remoteEntry.js",
      exposes: {
        "./remote-app": "./src/App.tsx",
      },
      shared: {
        react: { singleton: true, requiredVersion: "^19.0.0" },
        "react-dom": { singleton: true, requiredVersion: "^19.0.0" },
        "@mf-vite-ssr-example/shared-ui": { singleton: true },
      },
    }),
    react(),
  ],
  build: {
    target: "chrome89",
  },
  server: {
    fs: {
      allow: [".", "..", "../shared-ui"],
    },
    port: 4174,
    cors: true,
    origin: "http://localhost:4174",
  },
  preview: {
    port: 4174,
    cors: true,
  },
});
