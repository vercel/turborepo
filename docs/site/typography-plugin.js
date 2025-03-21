const plugin = require("tailwindcss/plugin");

const typographyPlugin = plugin(({ addUtilities }) => {
  const newUtilities = {
    // Heading variants
    ".text-heading-72": {
      fontSize: "4.5rem",
      lineHeight: "4.5rem",
      letterSpacing: "-4.32px",
      fontWeight: "600",
    },
    ".text-heading-64": {
      fontSize: "4rem",
      lineHeight: "4rem",
      letterSpacing: "-2.84px",
      fontWeight: "600",
    },
    ".text-heading-56": {
      fontSize: "3.5rem",
      lineHeight: "3.5rem",
      letterSpacing: "-3.36px",
      fontWeight: "600",
    },
    ".text-heading-48": {
      fontSize: "3rem",
      lineHeight: "3.5rem",
      letterSpacing: "-2.88px",
      fontWeight: "600",
    },
    ".text-heading-40": {
      fontSize: "2.5rem",
      lineHeight: "3rem",
      letterSpacing: "-2.4px",
      fontWeight: "600",
    },
    ".text-heading-32": {
      fontSize: "2rem",
      lineHeight: "2.5rem",
      letterSpacing: "-1.28px",
      fontWeight: "600",
    },
    ".text-heading-24": {
      fontSize: "1.5rem",
      lineHeight: "2rem",
      letterSpacing: "-0.96px",
      fontWeight: "600",
    },
    ".text-heading-20": {
      fontSize: "1.25rem",
      lineHeight: "1.625rem",
      letterSpacing: "-0.4px",
      fontWeight: "600",
    },
    ".text-heading-16": {
      fontSize: "1rem",
      lineHeight: "1.5rem",
      letterSpacing: "-0.32px",
      fontWeight: "600",
    },
    ".text-heading-14": {
      fontSize: "0.875rem",
      lineHeight: "1.25rem",
      letterSpacing: "-0.28px",
      fontWeight: "600",
    },
    // Button variants
    ".text-button-16": {
      fontSize: "1rem",
      lineHeight: "1.25rem",
      fontWeight: "500",
    },
    ".text-button-14": {
      fontSize: "0.875rem",
      lineHeight: "1.25rem",
      fontWeight: "500",
    },
    ".text-button-12": {
      fontSize: "0.75rem",
      lineHeight: "1rem",
      fontWeight: "500",
    },
    // Label variants
    ".text-label-20": {
      fontSize: "1.25rem",
      lineHeight: "2rem",
      fontWeight: "400",
    },
    ".text-label-18": {
      fontSize: "1.125rem",
      lineHeight: "1.25rem",
      fontWeight: "400",
    },
    ".text-label-16": {
      fontSize: "1rem",
      lineHeight: "1.25rem",
    },
    ".text-label-14": {
      fontSize: "0.875rem",
      lineHeight: "1.25rem",
      fontWeight: "400",
    },
    ".text-label-13": {
      fontSize: "0.8125rem",
      lineHeight: "1rem",
      fontWeight: "400",
    },
    ".text-label-12": {
      fontSize: "0.75rem",
      lineHeight: "1rem",
      fontWeight: "400",
    },
    // Copy variants
    ".text-copy-24": {
      fontSize: "1.5rem",
      lineHeight: "2.25rem",
      fontWeight: "400",
    },
    ".text-copy-20": {
      fontSize: "1.25rem",
      lineHeight: "2.25rem",
      fontWeight: "400",
    },
    ".text-copy-18": {
      fontSize: "1.125rem",
      lineHeight: "1.75rem",
      fontWeight: "400",
    },
    ".text-copy-16": {
      fontSize: "1rem",
      lineHeight: "1.5rem",
      fontWeight: "400",
    },
    ".text-copy-14": {
      fontSize: "0.875rem",
      lineHeight: "1.25rem",
      fontWeight: "400",
    },
    ".text-copy-13": {
      fontSize: "0.8125rem",
      lineHeight: "1.125rem",
      fontWeight: "400",
    },
  };

  addUtilities(newUtilities);
});

module.exports = typographyPlugin;
