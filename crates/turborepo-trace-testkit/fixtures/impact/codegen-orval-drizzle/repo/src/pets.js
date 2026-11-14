const { getPet } = require('./api/petstore');

module.exports = { firstPet: () => getPet(1) };
