import { federation } from "@module-federation/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig } from "vite";

export default defineConfig({
  nitro: {
    // Keep react/react-dom as Node externals in the Nitro SSR bundle so all
    // server-side code shares the same require() module instance via Node's
    // CJS module cache. Without this, Nitro bundles React inline and the
    // remote's react instance diverges, breaking hooks and context.
    traceDeps: [
      "react",
      "react-dom",
      "@module-federation/runtime",
      "@module-federation/runtime-core",
      "@module-federation/sdk",
    ],
  },
  plugins: [
    federation({
      dts: false,
      name: "host",
      hostInitInjectLocation: "entry",
      remotes: {
        remote: {
          type: "module",
          name: "remote",
          entry: "http://localhost:4174/remoteEntry.js",
        },
      },
      shared: {
        react: { singleton: true, requiredVersion: "^19.0.0" },
        "react-dom": { singleton: true, requiredVersion: "^19.0.0" },
        "@mf-vite-ssr-example/shared-ui": { singleton: true },
      },
    }),
    tanstackStart(),
    react(),
    nitro(),
  ],
  ssr: {
    optimizeDeps: {
      include: ["react", "react-dom"],
    },
  },
  build: {
    target: "chrome89",
  },
  server: {
    fs: {
      allow: [".", "..", "../shared-ui"],
    },
    port: 4173,
  },
});
