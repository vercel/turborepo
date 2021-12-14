import { hasExecutable } from "./hasExecutable";

export async function shouldUseYarn() {
  const userAgent = process.env.npm_config_user_agent;
  if (userAgent) {
    return Boolean(userAgent && userAgent.startsWith("yarn"));
  }

  return hasExecutable("yarn");
}
