export default class ComponentStyle {
  generateAndInjectStyles(
    executionContext,
    styleSheet,
    stylis
  ) {
    let dynamicHash = phash(this.baseHash, stylis.hash);
    let css = '';

    for (let i = 0; i < this.rules.length; i++) {
      const partRule = this.rules[i];

      if (typeof partRule === 'string') {
        css += partRule;

        if (process.env.NODE_ENV !== 'production') dynamicHash = phash(dynamicHash, partRule);
      } else if (partRule) {
        const partString = joinStringArray(
          flatten(partRule, executionContext, styleSheet, stylis)
        );
        dynamicHash = phash(dynamicHash, partString);
        css += partString;
      }
    }

    if (css) {
      const name = generateName(dynamicHash >>> 0);

      if (!styleSheet.hasNameForId(this.componentId, name)) {
        styleSheet.insertRules(
          this.componentId,
          name,
          stylis(css, `.${name}`, undefined, this.componentId)
        );
      }

      names = joinStrings(names, name);
    }
  }
}