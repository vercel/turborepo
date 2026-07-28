import { githubChannel } from "eve/channels/github";

export default githubChannel({
  botName: process.env.GITHUB_APP_SLUG
});
