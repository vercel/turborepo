const { add, mul } = require("math");

function total(price, count, shipping) {
  return add(mul(price, count), shipping);
}

module.exports = { total };
