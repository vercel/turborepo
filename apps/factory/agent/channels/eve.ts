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
    // Lets the operator page open an ad-hoc chat session from the browser.
    operatorConsole(),
    // Open on localhost for `eve dev` and the REPL; ignored in production.
    localDev(),
    // Lets the eve TUI and Vercel deployments reach the deployed agent.
    vercelOidc()
  ]
});
