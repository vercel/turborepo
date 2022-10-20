import { DiscordIcon, GitHubIcon } from "nextra/icons";
import { useConfig } from "nextra-theme-docs";

function Github() {
  const { project } = useConfig();
  return (
    <a
      href={project.link}
      className="hidden sm:flex p-2 text-current"
      target="_blank"
      rel="noreferrer"
    >
      <GitHubIcon />
      <span className="sr-only">Github</span>
    </a>
  );
}

function Discord() {
  const { chat } = useConfig();
  return (
    <a
      href={chat.link}
      className="hidden sm:flex p-2 text-current"
      target="_blank"
      rel="noreferrer"
    >
      <DiscordIcon />
      <span className="sr-only">Discord</span>
    </a>
  );
}

export { Github, Discord };
