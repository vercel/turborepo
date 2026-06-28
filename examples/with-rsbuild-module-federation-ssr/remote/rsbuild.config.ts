import { createRequire } from "node:module";
import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { dependencies } from "./package.json";

const require = createRequire(import.meta.url);

const shared = {
  "@mf-rsbuild-ssr-example/shared-ui": {
    singleton: true,
    requiredVersion: "1.0.0",
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
};

export default defineConfig({
  html: {
    favicon: "./public/favicon.ico",
  },
  server: {
    port: 3001,
  },
  plugins: [
    pluginReact(),
    pluginModuleFederation({
      dts: false,
      name: "remote",
      filename: "remoteEntry.js",
      experiments: {
        asyncStartup: true,
      },
      exposes: {
        "./remote-app": "./src/App.tsx",
      },
      remotes: {},
      shared,
    }),
    pluginModuleFederation(
      {
        dts: false,
        name: "remote",
        filename: "remoteEntry.js",
        experiments: {
          asyncStartup: true,
        },
        library: { type: "commonjs-module", name: "remote" },
        runtimePlugins: [
          require.resolve("@module-federation/node/runtimePlugin"),
        ],
        exposes: {
          "./remote-app": "./src/App.tsx",
        },
        remotes: {},
        shared,
      },
      {
        target: "node",
        environment: "node",
      },
    ),
  ],
  environments: {
    web: {
      source: {
        entry: {
          index: "./src/index.tsx",
        },
      },
      output: {
        target: "web",
      },
    },
    node: {
      source: {
        entry: {
          index: "./src/node.ts",
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
