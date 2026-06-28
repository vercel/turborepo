import React from "react";
import { hydrateRoot } from "react-dom/client";
import Remote from "remote/remote-app";
import App from "./App";

hydrateRoot(
  document.getElementById("root") as HTMLElement,
  <React.StrictMode>
    <App Remote={Remote} />
  </React.StrictMode>,
);
