export function encode(params: Record<string, unknown>): string {
  const qs = Object.keys(params)
    .filter((key) => params[key] != null)
    .map((key) => `${key}=${encodeURIComponent(String(params[key]))}`)
    .join('&');
  return qs ? `?${qs}` : '';
}
