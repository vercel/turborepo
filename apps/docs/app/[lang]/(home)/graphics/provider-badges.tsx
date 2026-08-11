import {
  SiCircleci,
  SiGithub,
  SiGitlab,
  SiJenkins,
  SiVercel,
} from "@icons-pack/react-simple-icons";

const providers = [
  { label: "GitHub", Logo: SiGithub },
  { label: "GitLab", Logo: SiGitlab },
  { label: "Vercel", Logo: SiVercel },
  { label: "Jenkins", Logo: SiJenkins },
  { label: "CircleCI", Logo: SiCircleci },
];

export function ProviderBadges() {
  return (
    <div className="flex size-full items-center justify-center overflow-hidden">
      <div className="flex items-center px-6">
        {providers.map(({ label, Logo }) => (
          <span
            className="-ml-3 flex size-16 items-center justify-center rounded-xl bg-background-100 shadow-(--ds-shadow-border-small) first:ml-0 lg:-ml-4 lg:size-20 lg:rounded-3xl"
            key={label}
            title={label}
          >
            <Logo
              aria-hidden="true"
              className="text-gray-1000"
              color="currentColor"
              size={32}
            />
          </span>
        ))}
      </div>
    </div>
  );
}
