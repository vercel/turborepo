export function onServer(): boolean {
  return typeof document === 'undefined';
}
