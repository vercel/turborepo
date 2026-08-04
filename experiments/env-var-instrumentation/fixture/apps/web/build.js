// Writes an artifact whose content embeds API_URL, and whose *shape* (not
// content) depends on MINIFY -- a control-flow dependency with no value leak.
const fs = require('fs');
fs.mkdirSync('dist', { recursive: true });
let body = `endpoint=${process.env.API_URL}\nhello world\n`;
if (process.env.MINIFY) body = body.replace(/\s+/g, ' ').trim();
fs.writeFileSync('dist/out.txt', body);
console.log('web build done');
