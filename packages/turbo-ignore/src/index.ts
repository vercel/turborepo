#!/usr/bin/env node

import turboIgnore from "./ignore";
import parseArgs from "./args";

turboIgnore({ args: parseArgs({ argv: process.argv.slice(2) }) });
