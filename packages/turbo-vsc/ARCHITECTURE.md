# Architecture

This document attempts to give a high level overview to the extension / LSP,
which APIs it uses to achieve its feature set, and how it is structured.

## Client

You're here! The client is the side that runs in VSCode. It is essentially
an entry point into the LSP but there are a few other things it manages
mostly for convience sake.

- basic syntax highlighting for the pipeline gradient
- discovery and installation of global / local turbo
- toolbar item to enable / disable the daemon
- some editor commands
  - start deamon
  - stop daemon
  - restart daemon
  - run turbo command
  - run turbo lint

Otherwise it simply selects the correct LSP binary and runs it using vscode's
built-in LSP library, and the LSP in turn interacts with the turbo daemon to
get the information it needs to fulfil client requests.

## Daemon

The daemon plays a minor role in relaying important metadata about the
workspace itself back to the LSP. The general rule is the LSP can know about
packages, turbo jsons, and how to parse them, but shouldn't need to do any
inference, package manager work, etc etc. Any heavy lifting should be kept
on the daemon.

## Server - turborepo_lsp

This is the rust side. It imports some parts from the rest of the turbo
codebase and utilizes the daemon to query data about the repository. When
the LSP is initialized, the client sends a list of open workspaces and the
LSP opens a connection to the (hopefully running) daemon, or starts one.

> Note that we use the `jsonc_parser` crate rather than turbo's own
> TurboJSON parsing logic for maximum flexibility. we don't care if parts
> of it are malformed, as long as we can parse the parts we need to perform
> the client's request. See the tech debt section for more.

### Language Server Protocol

Using the [`tower_lsp`](https://crates.io/crates/tower-lsp) crate makes LSPs
quite easy. It includes a server trait and all the message types required. We
implement the `LanguageServer` trait and the library is responsible for all
IO. All that remains is adding the relevant LSP handlers to support the
features we need, which are broken down below, and hooking it up to the
default communication mechanism: stdio.

To begin a session, the client sends an `initialize` request. The server must
respond with the capabilities it supports, such that the client knows what
type of questions it can ask. The capacilities we support are covered in the
next sections.

#### LanguageServer::did_open - textDocument/didOpen

We need to respond to updates to the turbo.json file live as the user is
writing in them. Watching the FS does not cut it, as we need to give context
to the _state of the buffer_ not the _file_. The LSP has support for this.
By advertising to the client the 'text document sync' capability, the client
knows that it can send us updates about file state. We send this during
initialization and as a result the client pushes events (open and change) when
turbo json files are loaded into the buffer.

We need to store these locally since later requests will only send the URI of
the resource they are querying. It is up to the LSP to store the current state
which we do through ropes.

All file changes (opening or changing) trigger `handle_file_update`, which is
detailed in the next part.

#### LanguageServer::did_change - textDocument/didChange

Updates are treated the same. Any file changes require flushing new diagnostics
which is done in `handle_file_update`. It is in charge of a few things:

- fetch a fresh list of packages and workspaces from the daemon (cheap)
- traverse the workspaces and parse the package name + scripts
- parse the turbo json to ensure
  - all globs are valid (global and pipeline specific ones)
  - all pipeline key names refer to valid tasks
  - all dependOn fields are sound

These are reported back to the client along with their line and column ranges
via the `textDocument/publishDiagnostics` LSP command and displayed on the
document.

#### LanguageServer::completion - textDocument/completion

If an IDE requests completion information at a particular position in the
document, this call kicks in. In VSCode, knowing that both the json LSP
and turbo LSP apply, will fire off a request to each and resolve JsonSchema
items, as well as turbo tasks. The logic for turbo is handled here.
To resolve, we get all referenced scripts in any package in the workspace,
as well as all valid `<package>#<script>` combinations, using the daemon
package discovery to do this. The FIELD completion kind ensures that the
task ids will only be recommended as keys.

Ordering / filtering is handled client side.

#### LanguageServer::code_lens - textDocument/codeLens

Code lenses are those handy little inline buttons you can find decorating
tests or main functions. Clicking a code lens triggers some operation defined
by the LSP. The only code lens that is useful to use is running turbo commands
so we provide a nice way to do that. The LSP informs the client that upon
clicking the lens it should invoke some command (`turbo run`) with some
particular arguments (the task that was clicked on).

#### LanguageServer::code_actions - textDocument/codeActions

Code actions allow editors to surface automatic fixes for diagnostics. The
client submits to the server the list of diagnostics it is
interested in providing actions for (likely the ones on screen) and we
can use the diagnostic code we issued earlier to identify an action, such
as running a particular codemod to fix a `deprecated:env-var` error.

#### LanguageServer::references - textDocument/references

Finally, we support the references capability. References allow clients to
request information about where a particular variable is used elsewhere in the
code. Translated to turborepo, it is 'what scripts and packages does this
pipeline entry refer to?'. This also helps with discoverability. General
method is as follows:

- parse the turbo json to find which pipeline item we requested data on
- search through all the workspaces
  - parse the package jsons
  - find scripts that match the pipeline item
- yield matching workspaces

## Tech Debt Notes

- we could consider moving the client side commands into the LSP to help with
  cross platform (editor) support.
- rather than watch package.json buffers, we simply read them from disk. this
  means that references may be incorrect for json files that the daemon does
  not know about. we can mitigate this slightly but until the daemon supports
  the LSP there is not a great workaround so the LSP is 'as bad as' the daemon.
- similar to above, we only cache file contents for `turbo.json` files and not
  any `package.json` files. This means we parse these fresh each time when a
  `turbo.json` changes, or when lenses and completions are requested. Parsing
  these is fast and keeps the LSP stateless, but we could consider a cache
- we should probably factor out and re-use logic and types regarding tasks
  rather than parsing and using an AST. it was done this way since at the time
  of writing, parsing a TurboJson using the rust code made tracing fields back
  to line numbers rough and more importantly fallible. in the case of the LSP
  we want to be as fault tolerant as possible. this _may_ have changed with
  the new error handling work
- we can probably evict files from our rope store once they are closed
