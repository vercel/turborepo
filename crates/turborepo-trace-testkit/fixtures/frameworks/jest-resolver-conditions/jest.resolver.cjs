// One dependency has only an `import` entry: resolve it with that condition added.
module.exports = (id, ctx) =>
  ctx.defaultResolver(id, {
    ...ctx,
    conditions: id === 'esm-only-pkg' ? [...(ctx.conditions ?? []), 'import'] : ctx.conditions,
  });
