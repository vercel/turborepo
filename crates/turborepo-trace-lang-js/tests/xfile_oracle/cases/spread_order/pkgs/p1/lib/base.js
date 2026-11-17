// What the spread brings is decided at run time.
export function base() {
  return JSON.parse(process.env.FLOW_BASE || '{"name":"./mods/b","other":"./mods/c"}');
}
