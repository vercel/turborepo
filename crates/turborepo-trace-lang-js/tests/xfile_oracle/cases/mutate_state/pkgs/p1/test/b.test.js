import { load, setName } from '../lib/helper';
test('t', () => {
  setName('./mods/b');
  load();
});
