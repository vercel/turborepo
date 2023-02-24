export default function foo() {
  if (!process.env.IS_SERVER) {
    return "bar";
  }
  return "foo";
}
