require('@ws/lib'); // pnpm `workspace:` link: first party
require('@ws/hoisted'); // npm/yarn-style link in the root node_modules: first party
require('dep'); // real package in node_modules/.pnpm: external
require('linked'); // link to a directory outside the workspace: external
