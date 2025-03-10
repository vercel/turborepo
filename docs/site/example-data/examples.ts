export interface Example {
  slug: string;
  name: string;
  description: string;
  template?: string;
  featured?: boolean;
  boost?: boolean;
  maintainedByCoreTeam?: true;
}

export const EXAMPLES: Example[] = [
  {
    slug: "basic",
    name: "Next.js",
    description: "Minimal Turborepo example for learning the fundamentals.",
    template: "https://vercel.com/templates/next.js/turborepo-next-basic",
    featured: true,
    boost: true,
    maintainedByCoreTeam: true,
  },
  {
    slug: "design-system",
    name: "Design System",
    description:
      "Unify your site's look and feel by sharing a design system across multiple apps.",
    template: "https://vercel.com/templates/react/turborepo-design-system",
    featured: true,
  },
  {
    slug: "kitchen-sink",
    name: "Kitchen Sink",
    description:
      "Want to see a more in-depth example? Includes multiple frameworks, both frontend and backend.",
    template: "https://vercel.com/templates/remix/turborepo-kitchensink",
    featured: true,
    maintainedByCoreTeam: true,
  },
  {
    slug: "non-monorepo",
    name: "Non-Monorepo",
    description:
      "Example of using Turborepo in a single project without workspaces",
    maintainedByCoreTeam: true,
  },
  {
    slug: "with-angular",
    name: "Angular",
    description: "Minimal Turborepo example using Angular.",
  },
  {
    slug: "with-berry",
    name: "Yarn Berry",
    description: "Minimal Turborepo example using Yarn Berry.",
  },
  {
    slug: "with-changesets",
    name: "Monorepo with Changesets",
    description:
      "Simple Next.js monorepo preconfigured to publish packages via Changesets",
  },
  {
    slug: "with-docker",
    name: "Docker",
    description:
      "Monorepo with an Express API and a Next.js App deployed with Docker utilizing turbo prune",
  },
  {
    slug: "with-gatsby",
    name: "Gatsby.js",
    description:
      "Monorepo with a Gatsby.js and a Next.js app both sharing a UI Library",
    template: "https://vercel.com/templates/gatsby/turborepo-gatsby-starter",
    featured: true,
  },
  {
    slug: "with-nestjs",
    name: "Nest.js",
    description: "Minimal Turborepo example with Nest.js.",
    featured: true,
    boost: true,
  },
  {
    slug: "with-npm",
    name: "npm package manager",
    description: "Minimal Turborepoe example using npm as a package manager.",
  },
  {
    slug: "with-prisma",
    name: "Prisma",
    description: "Monorepo with a Next.js App fully configured with Prisma",
  },
  {
    slug: "with-react-native-web",
    name: "React Native",
    description:
      "Simple React Native & Next.js monorepo with a shared UI library",
    featured: true,
    template: "https://vercel.com/templates/react/turborepo-design-system",
  },
  {
    slug: "with-rollup",
    name: "Rollup",
    description:
      "Monorepo with a single Next.js app sharing a UI library bundled with Rollup",
  },
  {
    slug: "with-shell-commands",
    name: "Turborepo only",
    description: "A Turborepo-only monorepo.",
    maintainedByCoreTeam: true,
  },
  {
    slug: "with-svelte",
    name: "SvelteKit",
    description: "Monorepo with multiple SvelteKit apps sharing a UI Library",
    featured: true,
    template: "https://vercel.com/templates/svelte/turborepo-sveltekit-starter",
    boost: true,
    maintainedByCoreTeam: true,
  },
  {
    slug: "with-tailwind",
    name: "Tailwind CSS",
    description:
      "Monorepo with multiple Next.js apps sharing a UI Library all using Tailwind CSS with a shared config",
    featured: true,
    maintainedByCoreTeam: true,
  },
  {
    slug: "with-typeorm",
    name: "TypeORM",
    description: "Monorepo with TypeORM",
  },
  {
    slug: "with-vite",
    name: "Vite",
    description:
      "Monorepo with multiple Vanilla JS apps bundled with Vite, sharing a UI Library",
  },
  {
    slug: "with-vue-nuxt",
    name: "Vue/Nuxt",
    description: "Monorepo with Vue and Nuxt, sharing a UI Library",
  },
  {
    slug: "with-yarn",
    name: "Yarn package manager",
    description: "Monorepo using Yarn 1 for package management",
  },
];
