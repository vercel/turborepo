import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { dependencies } from "./package.json";

export default defineConfig({
  html: {
    favicon: "./public/favicon.ico",
  },
  plugins: [
    pluginReact(),
    pluginModuleFederation({
      name: "host",
      remotes: {
        remote: "remote@http://localhost:3001/mf-manifest.json",
      },
      exposes: {},
      shared: {
        "@mf-rsbuild-example/shared-ui": {
          singleton: true,
        },
        react: {
          requiredVersion: dependencies.react,
          singleton: true,
        },
        "react-dom": {
          requiredVersion: dependencies["react-dom"],
          singleton: true,
        },
      },
    }),
  ],
});
