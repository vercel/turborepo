const base = require('./ra');
module.exports = { routes: [...base.routes, { handler: './handlers/h5' }] };
