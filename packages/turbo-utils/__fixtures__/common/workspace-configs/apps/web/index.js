export default function web() {
  if (!process.env.ENV_2) {
    return "bar";
  }
  return "foo";
}
