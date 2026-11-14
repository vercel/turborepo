// Every static ESM form.
import defaultExport from './default.js';
import { named, other as renamed } from "./named.js";
import * as namespace from './namespace.js';
import './side-effect.js';
import def, { a, b } from './mixed.js';
import def2, * as ns2 from './mixed-ns.js';
import {} from './empty-braces.js';
import { default as d, 'string-name' as s } from './string-names.js';
import json from './data.json' with { type: 'json' };
import legacy from './legacy.json' assert { type: 'json' };
import `./template-no-subst.js`;

export * from './star.js';
export * as starNs from './star-ns.js';
export { x, y as z } from './reexport.js';
export { default } from './reexport-default.js';
export { default as named2 } from './reexport-default-named.js';

// Not references.
export { a, b };
export default function () {}
export const value = 'import x from "./nope.js"';
const meta = import.meta.url;
const obj = { import: './nope.js', require: './nope.js', export: 1 };
obj.import('./nope.js');

// Dynamic imports.
const lazy = () => import('./lazy.js');
const withOptions = await import('./with-options.json', { with: { type: 'json' } });
const computed = await import(`./locales/${lang}.js`);
const concat = import('./pages/' + page);
