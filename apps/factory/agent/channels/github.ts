import { githubChannel } from "eve/channels/github";

import { githubCredentials } from "../lib/github.js";

export default githubChannel({
  botName: process.env.GITHUB_APP_SLUG ?? "turborepo-eve-agent",
  credentials: githubCredentials
});
