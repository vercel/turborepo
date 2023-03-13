# Special File Types

There are a few files that we'll need to read regularly, so we combine their interfaces here.

## Default Values

Each of these structs implements per-field defaults without `Option<>` as long as we do not need to distinguish between "set" and "unset". For example, these two `package.json` files are materially different per business logic, and as such are implemented as `Option<Workspaces>` instead of just defaulting to an empty `Workspaces::TopLevel(vec![])`.

```json
{
  "workspaces": []
}
```

```json
{}
```

Note that "enabling serialized output to be only of user-supplied values" _is_ a need to distinguish between "set" and "unset".
