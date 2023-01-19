const core = require("@actions/core");

const sweep = require("./sweep");

sweep.sweep().catch(core.setFailed);
