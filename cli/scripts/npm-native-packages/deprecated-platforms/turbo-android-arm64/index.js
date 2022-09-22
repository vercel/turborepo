//#!/usr/bin/env node

const msg = `This OS/architecture combination is not officially supported for Turborepo.
Please add to https://github.com/vercel/turborepo/discussions/1891 if this impacts your workflow
`;

throw new Error(msg);
