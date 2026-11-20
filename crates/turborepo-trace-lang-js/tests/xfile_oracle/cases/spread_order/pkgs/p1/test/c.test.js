import { base } from '../lib/base';
import { load } from '../lib/helper';
test('t', () => {
  // A computed key after the property can be the property: ./mods/c at run time.
  const key = process.env.FLOW_KEY || 'name';
  load({ ...base(), name: './mods/a', [key]: './mods/c' });
});
