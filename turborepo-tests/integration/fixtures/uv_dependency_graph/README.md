This workspace exercises dependencies resolved by uv that cannot be recovered
from a member's static `project.dependencies`: a Hatch metadata hook and a
dependency on another member's extra. `leaf-lib` checks that direct edges stay
distinct from transitive dependencies; `unrelated` checks invalidation scope.

Regenerate `uv.lock` with `uv lock` from this directory. The lockfile is generated
from the real Hatch hook, rather than handwritten metadata. Graph tests consume
it with `uv workspace metadata --frozen --offline`, as Turbo does.
