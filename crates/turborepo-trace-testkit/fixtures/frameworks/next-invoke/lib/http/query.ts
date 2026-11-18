export function toQuery(values: Record<string, unknown>): string {
  const parts = Object.entries(values)
    .filter(([, value]) => value !== undefined && value !== null)
    .map(([name, value]) => name + '=' + encodeURIComponent(String(value)));
  return parts.length > 0 ? '?' + parts.join('&') : '';
}
