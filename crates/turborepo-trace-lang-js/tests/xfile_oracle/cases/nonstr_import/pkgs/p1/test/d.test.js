import { load } from '../lib/helper';
test('t', () => {
  load([process.env.FLOW_K0 === '1' ? './mods/a.js' : './mods/b.js']);
});
