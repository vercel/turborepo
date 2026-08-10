import {
  SiCircleci,
  SiGithub,
  SiGitlab,
  SiJenkins,
  SiVercel,
} from "@icons-pack/react-simple-icons";

const providers = [
  { label: "GitHub", Logo: SiGithub, themeAware: true },
  { label: "GitLab", Logo: SiGitlab },
  { label: "Vercel", Logo: SiVercel, themeAware: true },
  { label: "Jenkins", Logo: SiJenkins, logoClassName: "dark:fill-gray-1000" },
  {
    label: "CircleCI",
    Logo: SiCircleci,
    logoClassName: "dark:fill-gray-1000",
  },
];

export function ProviderBadges() {
  return (
    <div className="flex size-full items-center justify-center overflow-hidden">
      <div className="flex items-center px-6">
        {providers.map(({ label, Logo, logoClassName, themeAware }) => (
          <span
            className="-ml-3 flex size-16 items-center justify-center rounded-xl bg-background-100 shadow-(--ds-shadow-border-small) first:ml-0 lg:-ml-4 lg:size-20 lg:rounded-3xl"
            key={label}
            title={label}
          >
            <Logo
              aria-hidden="true"
              className={themeAware ? "text-gray-1000" : logoClassName}
              color={themeAware ? "currentColor" : "default"}
              size={32}
            />
          </span>
        ))}
      </div>
    </div>
  );
}
