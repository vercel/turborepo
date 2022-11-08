import { DetailedFeatureLink } from "./Feature";
import { GitHubIcon } from "./Icons";

export const ExamplesArea = () => {
  return (
    <div className="grid grid-cols-1 mt-12 gap-x-6 gap-y-12 sm:grid-cols-2 lg:mt-16 lg:gap-x-8 lg:gap-y-12">
      <DetailedFeatureLink
        feature={{
          Icon: GitHubIcon,
          description: `Minimal Turborepo example for learning the
              fundamentals.`,
          name: "Basic",
        }}
        href="https://github.com/vercel/turbo/tree/main/examples/basic"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: GitHubIcon,
          description:
            "Unify your site's look and feel by sharing a design system across multiple apps.",
          name: "Design System",
        }}
        href="https://github.com/vercel/turbo/tree/main/examples/design-system"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: GitHubIcon,
          description:
            "Learn how to integrate with Tailwind, the popular CSS framework.",
          name: "With Tailwind CSS",
        }}
        href="https://github.com/vercel/turbo/tree/main/examples/with-tailwind"
      ></DetailedFeatureLink>
      <DetailedFeatureLink
        feature={{
          Icon: GitHubIcon,
          description:
            "Want to see a super-complex, kitchen-sink example? Includes multiple frameworks, both frontend and backend.",
          name: "Kitchen Sink",
        }}
        href="https://github.com/vercel/turbo/blob/main/examples/kitchen-sink"
      ></DetailedFeatureLink>
    </div>
  );
};
