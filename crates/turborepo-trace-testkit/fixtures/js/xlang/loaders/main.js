// A require wrapper.
const load = (name) => require(name);

console.log(load('./handlers/alpha').NAME, load('./handlers/beta').NAME);
