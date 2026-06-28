import { StartClient } from "@tanstack/react-start/client";
import type { ComponentType } from "react";
import { hydrateRoot } from "react-dom/client";
import "./routeTree.gen";
import { getRouter } from "./router";

const router = getRouter();
const Client = StartClient as ComponentType<{ router: typeof router }>;

hydrateRoot(document, <Client router={router} />);
