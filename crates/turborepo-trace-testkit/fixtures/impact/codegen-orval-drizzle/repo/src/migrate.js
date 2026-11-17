const fs = require('fs');
const path = require('path');

module.exports = () => fs.readFileSync(path.join(__dirname, '../drizzle/0000_init.sql'), 'utf8');
