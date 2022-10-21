import { DiscordIcon, GitHubIcon } from "nextra/icons";

function Github() {
  return (
    <a
      href="https://github.com/vercel/turbo"
      className="hidden sm:flex p-2 text-current hover:opacity-75"
      target="_blank"
      rel="noreferrer"
    >
      <GitHubIcon />
      <span className="sr-only">Github</span>
    </a>
  );
}

function Discord() {
  return (
    <a
      href="https://turborepo.org/discord"
      className="hidden sm:flex p-2 text-current hover:opacity-75"
      target="_blank"
      rel="noreferrer"
    >
      <DiscordIcon />
      <span className="sr-only">Discord</span>
    </a>
  );
}

export { Github, Discord };
