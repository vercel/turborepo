import { slackChannel } from "eve/channels/slack";

import { slackCredentials } from "../lib/slack.js";

export default slackChannel({
  credentials: slackCredentials
});
