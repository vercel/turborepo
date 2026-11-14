// Templates, concatenation and conditional alternatives.
const fs = require('fs');
const path = require('path');

const LANG = 'en';
const profile = process.env.XLANG_DEBUG ? 'debug' : 'release';
console.log(fs.readFileSync(`${__dirname}/locales/${LANG}.json`, 'utf8'));
console.log(fs.readFileSync(path.join(__dirname, 'profiles', profile + '.ini'), 'utf8'));
