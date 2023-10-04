export default function docs() {
  if (!process.env.ENV_1) {
    return "bar";
  }
  return "foo";
}
