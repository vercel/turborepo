export default async function start() {
  process.env.SUITE_BEGAN_AT = new Date().toISOString();
}
