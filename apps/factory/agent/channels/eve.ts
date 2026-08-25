import { type AuthFn, localDev, vercelOidc } from "eve/channels/auth";
import { eveChannel } from "eve/channels/eve";

import {
  isOperatorSessionRequest,
  OPERATOR_SESSION_PRINCIPAL
} from "../lib/operator-console.js";

function operatorSession(): AuthFn<Request> {
  return (request) =>
    isOperatorSessionRequest(request) ? OPERATOR_SESSION_PRINCIPAL : null;
}

export default eveChannel({
  auth: [
    // Authenticates browser access to durable workspace sessions.
    operatorSession(),
    // Open on localhost for `eve dev` and the REPL; ignored in production.
    localDev(),
    // Lets the eve TUI and Vercel deployments reach the deployed agent.
    vercelOidc()
  ]
});
