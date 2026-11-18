export default async function globalTeardown(): Promise<void> {
  delete process.env.E2E_STARTED;
}
