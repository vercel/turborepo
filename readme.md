# Turbo Tooling

Turbo Tooling is an umbrella term for a set of tools built upon a common Rust-based build engine for incremental and distributed computation known as turbo-tasks.

## turbo-tasks

turbo-tasks is a flexible abstraction that enables incremental and distributed functionality. It allows you to split your build process into composable tasks, where each task is the combination of a function and the values it receives as input. The results of tasks are cached, so on subsequent runs, tasks are only re-executed if their input values have changed. Tasks can also be distributed across CPU cores and eventually may run on remote machines. This leads to much faster build processes, especially for warm builds.

## Tools

-   **turbopack** is web bundler and spiritual successor to [webpack](https://github.com/webpack/webpack)

-   **node-file-trace** is a rewrite of [`@vercel/nft`](https://github.com/vercel/nft) powered by turbopack

## Contributing

See [contributing.md](/contributing.md) to get started and [architecture.md](/architecture.md) for an overview of how things work.
