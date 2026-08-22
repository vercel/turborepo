import { type AuthFn, localDev, vercelOidc } from "eve/channels/auth";
import { eveChannel } from "eve/channels/eve";

import {
  isOperatorChatRequest,
  OPERATOR_CHAT_PRINCIPAL
} from "../lib/operator-console.js";

function operatorConsole(): AuthFn<Request> {
  return (request) =>
    isOperatorChatRequest(request) ? OPERATOR_CHAT_PRINCIPAL : null;
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
