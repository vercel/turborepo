# Turborepo VSC Extension

This extension provides a handy interface into your turbo-enhanced monorepo.

## Features

### Global turbo installer

We recommend you use turbo globally to simplify running commands. The extension will prompt you to install it if you don't already have it available.

### Automatic start / stop for the turbo daemon

Turborepo uses a background task to make your builds lightning fast. Rather than start it when you invoke turbo, we can instead start it when you launch your editor, keeping things snappy.

### Simple daemon status controls

In the bottom left toolbar, you will find the current status of the daemon. Clicking on the button will toggle it on and off.

### Repository discovery tools

#### Find references for turbo tasks

![references](resources/references.png)

Every task in your pipeline can be followed to find its references.

#### One-click task execution

Every entry in your pipeline can be executed with a single click for quick debugging.

#### Glob validation

![globs](resources/globs.png)

Detect bad globs live while you write, and even get github copilot involved in resolving them.

#### Detect missing tasks and packages

Pipeline entries that do not refer to a valid package or task will be highlighted for you.

#### Contextual codemods

Got deprecated syntax? Know about immediately and automatically fix them with codemods.
