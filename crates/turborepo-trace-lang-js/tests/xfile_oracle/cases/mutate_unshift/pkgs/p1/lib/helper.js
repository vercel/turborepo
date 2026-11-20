// The helper adds an element to the caller's array before the loads.
export function load(list) {
  list.unshift('./mods/c');
  return list.map((n) => require(n));
}
