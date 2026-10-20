export default function web() {
  if (!process.env.ENV_2) {
    return "bar";
  }
  if (process.env.NX_DOT_ENV === undefined) {
    return "does not exist";
  }
  if (process.env.ROOT_DOT_ENV === undefined) {
    return "does not exist";
  }
  if (process.env.WEB_DOT_ENV === undefined) {
    return "does not exist";
  }
  return "foo";
}
