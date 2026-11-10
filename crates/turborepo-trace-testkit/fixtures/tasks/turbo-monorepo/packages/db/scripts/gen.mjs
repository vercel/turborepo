import fs from 'node:fs';
const schema = fs.readFileSync(new URL('../schema.json', import.meta.url), 'utf8');
fs.writeFileSync(new URL('../src/generated/client.ts', import.meta.url), `export const schema = ${schema};`);
