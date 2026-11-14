const feature = process.env.IMPACT_FEATURE || 'alpha';
// A computed require in first-party code: statically uncertain.
export const impl = require('./features/' + feature);
