// The module adds a route after the assignment of its exports.
module.exports = { routes: [{ handler: './handlers/h1' }] };
module.exports.routes.push({ handler: './handlers/h2' });
