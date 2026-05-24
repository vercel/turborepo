import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { dependencies } from "./package.json";

export default defineConfig({
  html: {
    favicon: "./public/rsbuild-favicon.png",
  },
  server: {
    port: 3001,
  },
  plugins: [
    pluginReact(),
    pluginModuleFederation({
      dts: false,
      name: "remote",
      exposes: {
        "./remote-app": "./src/App.tsx",
      },
      remotes: {},
      shared: {
        "@mf-rsbuild-example/shared-ui": {
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
  ],
});
