import { connectSlackCredentials } from "@vercel/connect/eve";
import { slackChannel } from "eve/channels/slack";

export default slackChannel({
  credentials: connectSlackCredentials(
    process.env.SLACK_CONNECT_UID ?? "slack/my-agent"
  )
});
