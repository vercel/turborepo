export default function docs() {
  if (process.env.ENV_1 === undefined) {
    return "does not exist";
  }
  return "exists";
}
