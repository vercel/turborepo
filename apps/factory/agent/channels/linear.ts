import { linearChannel } from "eve/channels/linear";

import { linearCredentials } from "../lib/linear.js";

export default linearChannel({
  credentials: linearCredentials
});
