// One dependency only exports an `import` entry: resolve it with that condition added.
module.exports = (request, options) =>
  options.defaultResolver(request, {
    ...options,
    conditions:
      request === 'tree-walker'
        ? [...(options.conditions ?? []), 'import']
        : options.conditions,
  });
