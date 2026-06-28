import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { dependencies } from "./package.json";

const shared = {
    "@mf-rsbuild-ssr-example/shared-ui": {
      singleton: true,
      requiredVersion: "1.0.0",
    },
  react: {
    requiredVersion: dependencies.react,
    singleton: true,
  },
  "react-dom": {
    requiredVersion: dependencies["react-dom"],
    singleton: true,
  },
};

export default defineConfig({
  html: {
    favicon: "./public/favicon.ico",
  },
  plugins: [
    pluginReact(),
    pluginModuleFederation({
      name: "host",
      dts: false,
      remotes: {
        remote: "remote@http://localhost:3001/mf-manifest.json",
      },
      exposes: {},
      shared,
    }),
  ],
  environments: {
    web: {
      source: {
        entry: {
          index: "./src/index.client.tsx",
        },
      },
      output: {
        target: "web",
      },
      html: {
        template: "./index.html",
      },
    },
    node: {
      source: {
        entry: {
          index: "./src/index.server.tsx",
        },
      },
      output: {
        target: "node",
        distPath: {
          root: "dist/server",
        },
        autoExternal: true,
      },
    },
  },
});
