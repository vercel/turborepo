---
title: Sitemap
description: A high-level semantic index of the Turborepo documentation for LLM-assisted navigation and quick orientation.
---

# Turborepo Documentation Sitemap

## Purpose

This file is a high-level semantic index of the documentation.
It is intended for:

- LLM-assisted navigation (ChatGPT, Claude, etc.)
- Quick orientation for contributors
- Identifying relevant documentation areas during development

It is not intended to replace individual docs.

---

- [Introduction](/docs)
    - Type: Overview
    - Summary: Welcome to the Turborepo documentation!
    - Prerequisites: None
    - Topics: introduction, overview

- [Telemetry](/docs/telemetry)
    - Type: Reference
    - Summary: Learn more about Turborepo's anonymous telemetry.
    - Prerequisites: None
    - Topics: telemetry, privacy

- [Community](/docs/community)
    - Type: Reference
    - Summary: Learn about the Turborepo community.
    - Prerequisites: None
    - Topics: community

- [Acknowledgements](/docs/acknowledgments)
    - Type: Reference
    - Summary: Thank you to all these developers, build systems, and monorepo tools for their support and assistance.
    - Prerequisites: None
    - Topics: acknowledgements, credits

- [Support policy](/docs/support-policy)
    - Type: Reference
    - Summary: Learn about Turborepo's Support policy.
    - Prerequisites: None
    - Topics: support, policy

## Getting Started

- [Getting started](/docs/getting-started)
    - Type: Tutorial
    - Summary: Get started with Turborepo.
    - Prerequisites: None
    - Topics: getting started, setup

    - [Installation](/docs/getting-started/installation)
        - Type: Tutorial
        - Summary: Learn how to get started with Turborepo.
        - Prerequisites: Getting started
        - Topics: installation, setup

    - [Add to an existing repository](/docs/getting-started/add-to-existing-repository)
        - Type: Tutorial
        - Summary: Using Turborepo with your existing repository
        - Prerequisites: Getting started
        - Topics: existing repository, migration

    - [Editor integration](/docs/getting-started/editor-integration)
        - Type: Tutorial
        - Summary: Making the most of Turborepo
        - Prerequisites: Getting started
        - Topics: editor, IDE, integration

    - [Start with an example](/docs/getting-started/examples)
        - Type: Tutorial
        - Summary: Start with an example Turborepo.
        - Prerequisites: Getting started
        - Topics: examples, templates

## Crafting Your Repository

- [Crafting your repository](/docs/crafting-your-repository)
    - Type: Conceptual
    - Summary: Design and build your Turborepo.
    - Prerequisites: Getting started
    - Topics: repository design, architecture

    - [Structuring a repository](/docs/crafting-your-repository/structuring-a-repository)
        - Type: Conceptual
        - Summary: Start by creating a repository using the conventions of the ecosystem.
        - Prerequisites: Crafting your repository
        - Topics: structure, conventions, monorepo

    - [Managing dependencies](/docs/crafting-your-repository/managing-dependencies)
        - Type: How-to
        - Summary: Learn how to manage dependencies in your monorepo's workspace.
        - Prerequisites: Crafting your repository
        - Topics: dependencies, package management

    - [Creating an Internal Package](/docs/crafting-your-repository/creating-an-internal-package)
        - Type: How-to
        - Summary: Learn how to create an Internal Package for your monorepo.
        - Prerequisites: Crafting your repository
        - Topics: internal packages, shared code

    - [Configuring tasks](/docs/crafting-your-repository/configuring-tasks)
        - Type: How-to
        - Summary: Learn how to describe the workflows in your repository to get them done as fast as possible.
        - Prerequisites: Crafting your repository
        - Topics: tasks, turbo.json, configuration

    - [Running tasks](/docs/crafting-your-repository/running-tasks)
        - Type: How-to
        - Summary: Learn how to run tasks in your repository through the turbo CLI.
        - Prerequisites: Configuring tasks
        - Topics: running tasks, CLI

    - [Caching](/docs/crafting-your-repository/caching)
        - Type: Conceptual
        - Summary: Learn about caching in Turborepo.
        - Prerequisites: Running tasks
        - Topics: caching, performance

    - [Developing applications](/docs/crafting-your-repository/developing-applications)
        - Type: How-to
        - Summary: Learn how to develop applications in your repository.
        - Prerequisites: Crafting your repository
        - Topics: development, dev server

    - [Using environment variables](/docs/crafting-your-repository/using-environment-variables)
        - Type: How-to
        - Summary: Learn how to handle environments for your applications.
        - Prerequisites: Crafting your repository
        - Topics: environment variables, env

    - [Constructing CI](/docs/crafting-your-repository/constructing-ci)
        - Type: How-to
        - Summary: Learn how Turborepo can help you efficiently complete all the necessary tasks and accelerate your development workflow.
        - Prerequisites: Caching
        - Topics: CI, continuous integration

    - [Upgrading](/docs/crafting-your-repository/upgrading)
        - Type: How-to
        - Summary: Learn how to upgrade turbo to get the latest improvements to your repository.
        - Prerequisites: None
        - Topics: upgrading, migration

    - [Understanding your repository](/docs/crafting-your-repository/understanding-your-repository)
        - Type: How-to
        - Summary: Learn how to understand your repository structure using Turborepo.
        - Prerequisites: Crafting your repository
        - Topics: repository analysis, debugging

## Core Concepts

- [Core concepts](/docs/core-concepts)
    - Type: Conceptual
    - Summary: Learn about the core concepts behind Turborepo.
    - Prerequisites: Getting started
    - Topics: concepts, fundamentals

    - [Package and Task Graphs](/docs/core-concepts/package-and-task-graph)
        - Type: Conceptual
        - Summary: Turborepo builds a Task Graph based on your configuration and repository structure.
        - Prerequisites: Core concepts
        - Topics: task graph, package graph, DAG

    - [Package types](/docs/core-concepts/package-types)
        - Type: Conceptual
        - Summary: Learn about the different types of packages in a workspace.
        - Prerequisites: Core concepts
        - Topics: package types, workspace

    - [Internal Packages](/docs/core-concepts/internal-packages)
        - Type: Conceptual
        - Summary: Learn how to build Internal Packages in your monorepo.
        - Prerequisites: Package types
        - Topics: internal packages, shared code

    - [Remote Caching](/docs/core-concepts/remote-caching)
        - Type: Conceptual
        - Summary: Share cache artifacts across machines for even faster builds.
        - Prerequisites: Caching
        - Topics: remote caching, Vercel

## Reference

- [Turborepo API reference](/docs/reference)
    - Type: Reference
    - Summary: Learn about Turborepo's APIs using the reference.
    - Prerequisites: None
    - Topics: API, reference

    - [Configuring turbo.json](/docs/reference/configuration)
        - Type: Reference
        - Summary: Learn how to configure Turborepo through turbo.json.
        - Prerequisites: None
        - Topics: configuration, turbo.json

    - [Package Configurations](/docs/reference/package-configurations)
        - Type: Reference
        - Summary: Learn how to use Package Configurations to bring greater task flexibility to your monorepo's package.
        - Prerequisites: Configuration
        - Topics: package configuration, turbo.json

    - [System environment variables](/docs/reference/system-environment-variables)
        - Type: Reference
        - Summary: Learn about system variables used by Turborepo.
        - Prerequisites: None
        - Topics: environment variables, system

    - [File glob specification](/docs/reference/globs)
        - Type: Reference
        - Summary: Learn about the file glob specification used by turbo.
        - Prerequisites: None
        - Topics: globs, patterns

    - [Options overview](/docs/reference/options-overview)
        - Type: Reference
        - Summary: Flags, configurations, and System Environment Variables for Turborepo
        - Prerequisites: None
        - Topics: options, flags, CLI

    - [run](/docs/reference/run)
        - Type: Reference
        - Summary: API reference for the turbo run command
        - Prerequisites: Running tasks
        - Topics: run, CLI

    - [watch](/docs/reference/watch)
        - Type: Reference
        - Summary: API reference for the watch command
        - Prerequisites: Running tasks
        - Topics: watch, development

    - [generate](/docs/reference/generate)
        - Type: Reference
        - Summary: API reference for the turbo generate command
        - Prerequisites: None
        - Topics: generate, scaffolding

    - [scan](/docs/reference/scan)
        - Type: Reference
        - Summary: API reference for the turbo scan command
        - Prerequisites: None
        - Topics: scan, analysis

    - [ls](/docs/reference/ls)
        - Type: Reference
        - Summary: API reference for the turbo ls command
        - Prerequisites: None
        - Topics: ls, list

    - [query](/docs/reference/query)
        - Type: Reference
        - Summary: API reference for the turbo query command
        - Prerequisites: None
        - Topics: query, GraphQL

    - [prune](/docs/reference/prune)
        - Type: Reference
        - Summary: API reference for the turbo prune command
        - Prerequisites: None
        - Topics: prune, Docker

    - [login](/docs/reference/login)
        - Type: Reference
        - Summary: API reference for the turbo login command
        - Prerequisites: Remote Caching
        - Topics: login, authentication

    - [logout](/docs/reference/logout)
        - Type: Reference
        - Summary: API reference for the turbo logout command
        - Prerequisites: Remote Caching
        - Topics: logout, authentication

    - [link](/docs/reference/link)
        - Type: Reference
        - Summary: API reference for the turbo link command
        - Prerequisites: Remote Caching
        - Topics: link, remote caching

    - [unlink](/docs/reference/unlink)
        - Type: Reference
        - Summary: API reference for the turbo unlink command
        - Prerequisites: Remote Caching
        - Topics: unlink, remote caching

    - [telemetry](/docs/reference/telemetry)
        - Type: Reference
        - Summary: API reference for the turbo telemetry command
        - Prerequisites: None
        - Topics: telemetry

    - [bin](/docs/reference/bin)
        - Type: Reference
        - Summary: API reference for the turbo bin command
        - Prerequisites: None
        - Topics: bin, path

    - [info](/docs/reference/info)
        - Type: Reference
        - Summary: API reference for the turbo info command
        - Prerequisites: None
        - Topics: info, diagnostics

    - [docs](/docs/reference/docs)
        - Type: Reference
        - Summary: API reference for the turbo docs command
        - Prerequisites: None
        - Topics: docs, documentation

    - [devtools](/docs/reference/devtools)
        - Type: Reference
        - Summary: API reference for the turbo devtools command
        - Prerequisites: None
        - Topics: devtools, debugging

    - [boundaries](/docs/reference/boundaries)
        - Type: Reference
        - Summary: API reference for the turbo boundaries command
        - Prerequisites: None
        - Topics: boundaries, constraints

    - [create-turbo](/docs/reference/create-turbo)
        - Type: Reference
        - Summary: Quickly set up a new Turborepo repository from scratch.
        - Prerequisites: None
        - Topics: create-turbo, scaffolding

    - [@turbo/codemod](/docs/reference/turbo-codemod)
        - Type: Reference
        - Summary: Learn more about how Turborepo uses codemods to make version migrations easy.
        - Prerequisites: None
        - Topics: codemod, migration

    - [@turbo/gen](/docs/reference/turbo-gen)
        - Type: Reference
        - Summary: Quickly generate new code in your Turborepo.
        - Prerequisites: None
        - Topics: generators, scaffolding

    - [turbo-ignore](/docs/reference/turbo-ignore)
        - Type: Reference
        - Summary: Learn how to use turbo-ignore to skip tasks in CI.
        - Prerequisites: CI
        - Topics: turbo-ignore, CI

    - [eslint-config-turbo](/docs/reference/eslint-config-turbo)
        - Type: Reference
        - Summary: Learn more about eslint-config-turbo.
        - Prerequisites: None
        - Topics: ESLint, configuration

    - [eslint-plugin-turbo](/docs/reference/eslint-plugin-turbo)
        - Type: Reference
        - Summary: Learn more about eslint-plugin-turbo.
        - Prerequisites: None
        - Topics: ESLint, plugin

## Guides

- [Guides](/docs/guides)
    - Type: How-to
    - Summary: Learn how to use your favorite tooling in a Turborepo.
    - Prerequisites: Getting started
    - Topics: guides, tutorials

    - [Generating code](/docs/guides/generating-code)
        - Type: How-to
        - Summary: Learn how to generate code using Turborepo.
        - Prerequisites: Guides
        - Topics: code generation, scaffolding

    - [Handling platforms](/docs/guides/handling-platforms)
        - Type: How-to
        - Summary: Learn how to handle caching around operating systems, architectures, and other arbitrary conditions for Turborepo tasks.
        - Prerequisites: Caching
        - Topics: platforms, cross-platform

    - [Microfrontends](/docs/guides/microfrontends)
        - Type: How-to
        - Summary: Learn how to use Turborepo's built-in microfrontends proxy for local development.
        - Prerequisites: Guides
        - Topics: microfrontends, proxy

    - [Migrating from Nx](/docs/guides/migrating-from-nx)
        - Type: How-to
        - Summary: Learn how to migrate to Turborepo from Nx.
        - Prerequisites: Getting started
        - Topics: migration, Nx

    - [Multi-language support](/docs/guides/multi-language)
        - Type: How-to
        - Summary: Learn how to use multiple languages with Turborepo.
        - Prerequisites: Guides
        - Topics: multi-language, polyglot

    - [Publishing libraries](/docs/guides/publishing-libraries)
        - Type: How-to
        - Summary: Learn how to publish libraries to the npm registry from a monorepo.
        - Prerequisites: Guides
        - Topics: publishing, npm, libraries

    - [Single-package workspaces](/docs/guides/single-package-workspaces)
        - Type: How-to
        - Summary: Learn how to use Turborepo in a single-package workspace.
        - Prerequisites: Getting started
        - Topics: single package, workspace

    - [Skipping tasks](/docs/guides/skipping-tasks)
        - Type: How-to
        - Summary: Never do the same work twice.
        - Prerequisites: Running tasks
        - Topics: skipping tasks, optimization

### Tools

- [Tools](/docs/guides/tools)
    - Type: How-to
    - Summary: Learn how to use your favorite tools in a monorepo.
    - Prerequisites: Guides
    - Topics: tools, integrations

    - [Biome](/docs/guides/tools/biome)
        - Type: How-to
        - Summary: Learn how to use Biome in your Turborepo projects.
        - Prerequisites: Tools
        - Topics: Biome, linting, formatting

    - [Docker](/docs/guides/tools/docker)
        - Type: How-to
        - Summary: Learn how to use Docker in a monorepo.
        - Prerequisites: Tools
        - Topics: Docker, containers

    - [ESLint](/docs/guides/tools/eslint)
        - Type: How-to
        - Summary: Learn how to use ESLint in a monorepo.
        - Prerequisites: Tools
        - Topics: ESLint, linting

    - [Jest](/docs/guides/tools/jest)
        - Type: How-to
        - Summary: Learn how to use Jest in a Turborepo.
        - Prerequisites: Tools
        - Topics: Jest, testing

    - [Oxc (oxlint and oxfmt)](/docs/guides/tools/oxc)
        - Type: How-to
        - Summary: Learn how to use oxlint and oxfmt in your Turborepo projects.
        - Prerequisites: Tools
        - Topics: Oxc, oxlint, oxfmt, linting

    - [Playwright](/docs/guides/tools/playwright)
        - Type: How-to
        - Summary: Learn how to use Playwright in a Turborepo.
        - Prerequisites: Tools
        - Topics: Playwright, E2E testing

    - [Prisma](/docs/guides/tools/prisma)
        - Type: How-to
        - Summary: Learn how to use Prisma in a Turborepo.
        - Prerequisites: Tools
        - Topics: Prisma, database

    - [shadcn/ui](/docs/guides/tools/shadcn-ui)
        - Type: How-to
        - Summary: Learn how to use shadcn/ui in a Turborepo.
        - Prerequisites: Tools
        - Topics: shadcn/ui, components

    - [Storybook](/docs/guides/tools/storybook)
        - Type: How-to
        - Summary: Learn how to use Storybook in a Turborepo.
        - Prerequisites: Tools
        - Topics: Storybook, components

    - [Tailwind CSS](/docs/guides/tools/tailwind)
        - Type: How-to
        - Summary: Learn how to use Tailwind CSS in a Turborepo.
        - Prerequisites: Tools
        - Topics: Tailwind, CSS

    - [TypeScript](/docs/guides/tools/typescript)
        - Type: How-to
        - Summary: Learn how to use TypeScript in a monorepo.
        - Prerequisites: Tools
        - Topics: TypeScript

    - [Vitest](/docs/guides/tools/vitest)
        - Type: How-to
        - Summary: Learn how to use Vitest in a monorepo.
        - Prerequisites: Tools
        - Topics: Vitest, testing

### Frameworks

- [Frameworks](/docs/guides/frameworks)
    - Type: How-to
    - Summary: Integrate your favorite framework into Turborepo.
    - Prerequisites: Guides
    - Topics: frameworks, integrations

    - [Next.js](/docs/guides/frameworks/nextjs)
        - Type: How-to
        - Summary: Learn how to use Next.js in a monorepo.
        - Prerequisites: Frameworks
        - Topics: Next.js, React

    - [Vite](/docs/guides/frameworks/vite)
        - Type: How-to
        - Summary: Learn more about using Vite in your monorepo.
        - Prerequisites: Frameworks
        - Topics: Vite

    - [SvelteKit](/docs/guides/frameworks/sveltekit)
        - Type: How-to
        - Summary: Learn more about using SvelteKit in your monorepo.
        - Prerequisites: Frameworks
        - Topics: SvelteKit, Svelte

    - [Nuxt](/docs/guides/frameworks/nuxt)
        - Type: How-to
        - Summary: Learn more about using Nuxt in your monorepo.
        - Prerequisites: Frameworks
        - Topics: Nuxt, Vue

    - [Framework bindings in libraries](/docs/guides/frameworks/framework-bindings)
        - Type: How-to
        - Summary: Learn how to create framework bindings in packages.
        - Prerequisites: Frameworks
        - Topics: framework bindings, libraries

### CI Vendors

- [Continuous Integration](/docs/guides/ci-vendors)
    - Type: How-to
    - Summary: Recipes for using Turborepo with Vercel, GitHub Actions, and other continuous integration providers.
    - Prerequisites: Constructing CI
    - Topics: CI, continuous integration

    - [GitHub Actions](/docs/guides/ci-vendors/github-actions)
        - Type: How-to
        - Summary: Learn how to use GitHub Actions with Turborepo.
        - Prerequisites: Continuous Integration
        - Topics: GitHub Actions, CI

    - [Vercel](/docs/guides/ci-vendors/vercel)
        - Type: How-to
        - Summary: Learn how to use Turborepo on Vercel.
        - Prerequisites: Continuous Integration
        - Topics: Vercel, deployment

    - [GitLab CI](/docs/guides/ci-vendors/gitlab-ci)
        - Type: How-to
        - Summary: Learn how to use GitLab CI with Turborepo.
        - Prerequisites: Continuous Integration
        - Topics: GitLab CI

    - [CircleCI](/docs/guides/ci-vendors/circleci)
        - Type: How-to
        - Summary: Learn how to use CircleCI with Turborepo.
        - Prerequisites: Continuous Integration
        - Topics: CircleCI

    - [Travis CI](/docs/guides/ci-vendors/travis-ci)
        - Type: How-to
        - Summary: How to use Travis CI with Turborepo to optimize your CI workflow
        - Prerequisites: Continuous Integration
        - Topics: Travis CI

    - [Buildkite](/docs/guides/ci-vendors/buildkite)
        - Type: How-to
        - Summary: Learn how to use Buildkite with Turborepo.
        - Prerequisites: Continuous Integration
        - Topics: Buildkite

## Error Messages

- [Recursive turbo invocations](/docs/messages/recursive-turbo-invocations)
    - Type: Troubleshooting
    - Summary: Learn more about errors with recursive scripts and tasks in Turborepo.
    - Prerequisites: None
    - Topics: errors, recursive

- [Invalid environment variable prefix](/docs/messages/invalid-env-prefix)
    - Type: Troubleshooting
    - Summary: Learn more about errors with invalid environment variable prefixes in Turborepo.
    - Prerequisites: None
    - Topics: errors, environment variables

- [Unnecessary package task syntax](/docs/messages/unnecessary-package-task-syntax)
    - Type: Troubleshooting
    - Summary: Learn more about errors with unnecessary package task syntax in Turborepo.
    - Prerequisites: None
    - Topics: errors, task syntax

- [Missing root task in turbo.json](/docs/messages/missing-root-task-in-turbo-json)
    - Type: Troubleshooting
    - Summary: Learn more about errors for missing root tasks in turbo.json in Turborepo.
    - Prerequisites: None
    - Topics: errors, configuration

- [Package task in single-package workspace](/docs/messages/package-task-in-single-package-workspace)
    - Type: Troubleshooting
    - Summary: Learn more about errors with package tasks in single-package workspaces.
    - Prerequisites: None
    - Topics: errors, single package
