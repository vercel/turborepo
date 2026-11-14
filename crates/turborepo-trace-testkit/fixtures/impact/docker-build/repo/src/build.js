// Runs inside the image: dist/out.js from src/data.json and assets/logo.txt.
const fs = require('fs');
const path = require('path');

const data = JSON.parse(fs.readFileSync(path.join(__dirname, 'data.json'), 'utf8'));
const logo = fs.readFileSync(path.join(__dirname, '../assets/logo.txt'), 'utf8').trim();
fs.mkdirSync(path.join(__dirname, '../dist'), { recursive: true });
fs.writeFileSync(path.join(__dirname, '../dist/out.js'), 'module.exports = ' + JSON.stringify({ ...data, logo }) + ';\n');
