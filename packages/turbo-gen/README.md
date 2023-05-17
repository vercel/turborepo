# `@turbo/gen`

> This package is currently in **beta**. Please report any issues you encounter, and give us feedback about your experience using it!

Easily extend your Turborepo with new apps, and packages. Create new empty workspaces, copy existing workspaces, add workspaces from remote sources (just like `create-turbo`!) or run custom generators defined using [Plop](https://plopjs.com/) configurations.

## Usage

```bash
Usage: @turbo/gen [options] [command]

Extend your Turborepo

Options:
  -v, --version                     Output the current version
  -h, --help                        Display help for command

Commands:
  run|r [options] [generator-name]  Run custom generators
  workspace|w [options]             Add a new package or app to your project
  help [command]                    display help for command
```

## Workspace

Extend your Turborepo with new apps or packages. Create new empty workspaces, copy existing workspaces, or add workspaces from remote sources (just like `create-turbo`!).

### Usage

#### Blank Workspace

```bash
@turbo/gen workspace
```

#### Copy a Local Workspace

```bash
@turbo/gen workspace --copy
```

#### Copy a Remote Workspace

```bash
@turbo/gen workspace -e <git-url>
```

## Run

Extend your Turborepo by running custom generators defined using [Plop](https://plopjs.com/) configurations.

### Usage

```bash
@turbo/gen [generator-name]
```

### Writing Generators

`@turbo/gen` will search the root of your monorepo, and every workspace for generators defined at:

```bash
turbo/generators/config.ts
```

**NOTE**: By default, generators are run from the _root_ of the _workspace_ where they are defined.
