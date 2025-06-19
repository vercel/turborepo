import { bar } from "./bar" with { type: "js" };

export default function foo() {
  if (!process.env.IS_CI) {
    return "bar";
  }
  return "foo";
}
