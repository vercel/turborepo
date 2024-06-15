const core = require("@actions/core");
const sweep = require("./sweep");

sweep.storeTimestamp().catch(core.setFailed);
