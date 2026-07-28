import { federation } from "@module-federation/vite";
import react from "@vitejs/plugin-react";
import { writeFileSync } from "fs";
import { defineConfig, loadEnv } from "vite";
import { dependencies } from "./package.json";

export default defineConfig(({ mode }) => {
  const selfEnv = loadEnv(mode, process.cwd());
  return {
    server: {
      fs: {
        allow: [".", "..", "../shared"],
      },
    },
    build: {
      target: "chrome89",
    },
    plugins: [
      {
        name: "generate-environment",
        options: function () {
          console.info("selfEnv", selfEnv);
          writeFileSync(
            "./src/environment.ts",
            `export default ${JSON.stringify(selfEnv, null, 2)};`
          );
        },
      },
      federation({
        dts: true,
        dev: { disableDynamicRemoteTypeHints: true, remoteHmr: true },
        filename: "remoteEntry.js",
        name: "remote",
        exposes: {
          "./remote-app": "./src/App.tsx",
        },
        remotes: {},
        shared: {
          "@mf-vite-example/shared-ui": {
            singleton: true,
          },
          react: {
            requiredVersion: dependencies.react,
            singleton: true,
          },
          "react/": {},
          "react-dom": {
            requiredVersion: dependencies["react-dom"],
            singleton: true,
          },
        },
      }),
      react(),
    ],
  };
});
