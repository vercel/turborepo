export default async function globalSetup(): Promise<void> {
  process.env.E2E_STARTED = String(Date.now());
}
