# Turborepo starter with shell commands

This Turborepo starter is maintained by the Turborepo core team. This template is great for issue reproductions and exploring building task graphs without frameworks.

## Using this example

Run the following command:

```sh
pnpm dlx create-turbo@2.10.10 -e with-shell-commands
```

### For bug reproductions

Giving the Turborepo core team a minimal reproduction is the best way to create a tight feedback loop for a bug you'd like to report.

Because most monorepos will rely on more tooling than Turborepo (frameworks, linters, formatters, etc.), it's often useful for us to have a reproduction that strips away all of this other tooling so we can focus _only_ on Turborepo's role in your repo. This example does exactly that, giving you a good starting point for creating a reproduction.

- Feel free to rename/delete packages for your reproduction so that you can be confident it most closely matches your use case.
- If you need to use a different package manager to produce your bug, run `pnpm dlx @turbo/workspaces@2.10.10 convert` to switch package managers.
- It's possible that your bug really **does** have to do with the interaction of Turborepo and other tooling within your repository. If you find that your bug does not reproduce in this minimal example and you're confident Turborepo is still at fault, feel free to bring that other tooling into your reproduction.

## What's inside?

This Turborepo includes the following packages:

### Apps and Packages

- `app-a`: A final package that depends on all other packages in the graph and has no dependents. This could resemble an application in your monorepo that consumes everything in your monorepo through its topological tree.
- `app-b`: Another final package with many dependencies. No dependents, lots of dependencies.
- `pkg-a`: A package that defines all three example tasks.
- `pkg-b`: A package that defines build and type-checking tasks, plus a prebuild step.
- `tooling-config`: A package to simulate a common configuration used for all of your repository. This could resemble a configuration for tools like TypeScript or ESLint that are installed into all of your packages.

### Some commands to try

Run these commands from the example's root directory. They use the repository's pinned version of `turbo`.

- `pnpm turbo build lint check-types`: Runs all tasks in the default graph.
- `pnpm turbo build`: Builds `app-a` and `app-b` in parallel.
- `pnpm turbo build --filter=app-a`: Builds only `app-a` and its dependencies.
- `pnpm turbo lint`: Runs lints in all packages in parallel.
