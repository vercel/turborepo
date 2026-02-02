import { SiGithub } from "@icons-pack/react-simple-icons";
import { github } from "@/geistdocs";
import { cn } from "@/lib/utils";
import { Button } from "../ui/button";

interface GitHubButtonProps {
  className?: string;
}

export const GitHubButton = ({ className }: GitHubButtonProps) => {
  if (!(github.owner && github.repo)) {
    return null;
  }

  const url = `https://github.com/${github.owner}/${github.repo}`;

  return (
    <Button
      asChild
      className={cn(className)}
      size="icon-sm"
      type="button"
      variant="ghost"
    >
      <a href={url} rel="noopener" target="_blank">
        <SiGithub className="size-4" />
      </a>
    </Button>
  );
};
