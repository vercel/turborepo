export function origin() {
  const host = process.env.PREVIEW_HOST;
  return host ? `https://${host}` : `http://127.0.0.1:${process.env.APP_PORT || 4000}`;
}
