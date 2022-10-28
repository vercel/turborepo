import LinkButton from "../../LinkButton";

type BenchmarkHeadingProps = {
  name: string;
  href: string;
};

export default function BenchmarkHeading({
  name,
  href,
}: BenchmarkHeadingProps) {
  const nameIdSlug = name.toLowerCase().split(/\s+/).join("-");

  return (
    <div className="flex items-start nx-mt-8">
      <div className="flex-auto">
        <h3 className="nx-font-semibold nx-tracking-tight nx-text-2xl">
          {name}
          <span id={nameIdSlug} className="nx-absolute -nx-mt-20"></span>
          <a href={`#${nameIdSlug}`} className="subheading-anchor"></a>
        </h3>
      </div>
      <LinkButton href={href} size="sm">
        View <span className="hidden md:inline">benchmark</span> source
      </LinkButton>
    </div>
  );
}
