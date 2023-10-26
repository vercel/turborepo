import { DiscordIcon, GitHubIcon } from "nextra/icons";

function Github() {
  return (
    <a
      className="hidden p-2 text-current sm:flex hover:opacity-75"
      href="https://github.com/vercel/turbo"
      rel="noreferrer"
      target="_blank"
      title="Turbo GitHub repo"
    >
      {/* Nextra icons have a <title> attribute providing alt text */}
      <GitHubIcon />
    </a>
  );
}

function Discord() {
  return (
    <a
      className="hidden p-2 text-current sm:flex hover:opacity-75"
      href="https://turbo.build/discord"
      rel="noreferrer"
      target="_blank"
      title="Turbo Discord server"
    >
      <DiscordIcon />
    </a>
  );
}

export { Github, Discord };
