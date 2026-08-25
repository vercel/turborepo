import { type AuthFn, localDev, vercelOidc } from "eve/channels/auth";
import { eveChannel } from "eve/channels/eve";

import {
  isOperatorChatRequest,
  operatorChatPrincipal
} from "../lib/operator-console.js";

function operatorConsole(): AuthFn<Request> {
  return (request) =>
    isOperatorChatRequest(request) ? operatorChatPrincipal(request) : null;
}

export default eveChannel({
  auth: [
    // Keeps legacy direct Eve operator-console sessions authenticated.
    operatorConsole(),
    // Open on localhost for `eve dev` and the REPL; ignored in production.
    localDev(),
    // Lets the eve TUI and Vercel deployments reach the deployed agent.
    vercelOidc()
  ]
});
