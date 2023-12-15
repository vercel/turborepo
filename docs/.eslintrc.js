module.exports = {
  root: true,
  extends: ["@turbo/eslint-config/next"],
  rules: {
    // Most of these rules should probably be on. Turning them off because they fail in many places
    // and we need to set aside time to make them work.
    "@typescript-eslint/no-explicit-any": "warn",
    "@typescript-eslint/no-floating-promises": "warn",
    "@typescript-eslint/no-implied-eval": "warn",
    "@typescript-eslint/no-unnecessary-condition": "warn",
    "@typescript-eslint/no-unsafe-argument": "warn",
    "@typescript-eslint/no-unsafe-assignment": "warn",
    "@typescript-eslint/no-unsafe-call": "warn",
    "@typescript-eslint/no-unsafe-member-access": "warn",
    "@typescript-eslint/no-unsafe-return": "warn",
    "@typescript-eslint/require-await": "warn",
    "eslint-comments/require-description": "warn",
    "import/no-default-export": "warn",
    "jsx-a11y/anchor-is-valid": "warn",
    "jsx-a11y/click-events-have-key-events": "warn",
    "jsx-a11y/no-redundant-roles": "warn",
    "jsx-a11y/no-static-element-interactions": "warn",
    "no-console": "warn",
    "no-undef": "warn",
    "react/no-unknown-property": "warn",
    "react/no-unstable-nested-components": "warn",
  },
};
