const PORT = process.env.PORT ?? '3000';

export function getBaseUrl(): string {
  if (process.env.DEPLOYMENT_URL) {
    return `https://${process.env.DEPLOYMENT_URL}`;
  }
  return `http://localhost:${PORT}`;
}
