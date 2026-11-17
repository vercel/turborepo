// The plugin is only loaded through a computed path; the loader calls its export with an
// argument that the plugin loads.
export function boot(name) {
  const m = require('./plugins/' + name);
  return m.setup('./mods/c');
}
