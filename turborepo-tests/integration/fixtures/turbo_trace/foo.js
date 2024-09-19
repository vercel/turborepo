export default function foo() {
  if (!process.env.IS_CI) {
    return "bar";
  }
  return "foo";
}
