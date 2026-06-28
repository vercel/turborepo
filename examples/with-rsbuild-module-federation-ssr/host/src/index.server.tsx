import { createInstance } from "@module-federation/enhanced/runtime";
import React from "react";
import ReactDOMServer from "react-dom/server";
import type { ComponentType } from "react";
import * as ReactDOM from "react-dom";
import App from "./App";

const mf = createInstance({
  name: "host",
  remotes: [
    {
      name: "remote",
      entry: "http://localhost:3001/server/remoteEntry.js",
    },
  ],
  shared: {
    react: {
      version: "19.2.4",
      scope: "default",
      lib: () => React,
      shareConfig: {
        singleton: true,
        requiredVersion: "^19.2.4",
      },
    },
    "react-dom": {
      version: "19.2.4",
      scope: "default",
      lib: () => ReactDOM,
      shareConfig: {
        singleton: true,
        requiredVersion: "^19.2.4",
      },
    },
    "@mf-rsbuild-ssr-example/shared-ui": {
      version: "1.0.0",
      scope: "default",
      shareConfig: {
        singleton: true,
        requiredVersion: "1.0.0",
      },
    },
  },
});

export async function render() {
  const remoteModule = (await mf.loadRemote("remote/remote-app")) as {
    default: ComponentType;
  };

  return ReactDOMServer.renderToString(
    <React.StrictMode>
      <App Remote={remoteModule.default} />
    </React.StrictMode>,
  );
}
