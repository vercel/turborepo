import { PackageManager } from "../src/types";

const PACKAGE_MANAGERS: Array<PackageManager> = ["pnpm", "npm", "yarn"];
const REPO_TYPES = ["monorepo", "non-monorepo"];
const BOOLEAN_OPTIONS = [true, false];

export function generateConvertMatrix() {
  const matrix = [];
  for (const fixtureManager of PACKAGE_MANAGERS) {
    for (const fixtureType of REPO_TYPES) {
      for (const toManager of PACKAGE_MANAGERS) {
        for (const interactive of BOOLEAN_OPTIONS) {
          for (const dry of BOOLEAN_OPTIONS) {
            for (const install of BOOLEAN_OPTIONS) {
              matrix.push({
                fixtureManager,
                fixtureType,
                toManager,
                interactive,
                dry,
                install,
              });
            }
          }
        }
      }
    }
  }
  return matrix;
}

export function generateDetectMatrix() {
  const matrix = [];
  for (const project of PACKAGE_MANAGERS) {
    for (const manager of PACKAGE_MANAGERS) {
      for (const type of REPO_TYPES) {
        matrix.push({
          project,
          manager,
          type,
          result: project === manager,
        });
      }
    }
  }
  return matrix;
}

export function generateCreateMatrix() {
  const matrix = [];
  for (const project of PACKAGE_MANAGERS) {
    for (const manager of PACKAGE_MANAGERS) {
      for (const type of REPO_TYPES) {
        for (const interactive of BOOLEAN_OPTIONS) {
          for (const dry of BOOLEAN_OPTIONS) {
            matrix.push({
              project,
              manager,
              type,
              interactive,
              dry,
            });
          }
        }
      }
    }
  }
  return matrix;
}

export function generateReadMatrix() {
  const matrix = [];
  for (const fixtureManager of PACKAGE_MANAGERS) {
    for (const fixtureType of REPO_TYPES) {
      for (const toManager of PACKAGE_MANAGERS) {
        matrix.push({
          fixtureManager,
          fixtureType,
          toManager,
          shouldThrow: fixtureManager !== toManager,
        });
      }
    }
  }

  return matrix;
}

export function generateRemoveMatrix() {
  const matrix = [];
  for (const fixtureManager of PACKAGE_MANAGERS) {
    for (const fixtureType of REPO_TYPES) {
      for (const toManager of PACKAGE_MANAGERS) {
        for (const withNodeModules of BOOLEAN_OPTIONS) {
          for (const interactive of BOOLEAN_OPTIONS) {
            for (const dry of BOOLEAN_OPTIONS) {
              matrix.push({
                fixtureManager,
                fixtureType,
                withNodeModules,
                toManager,
                interactive,
                dry,
              });
            }
          }
        }
      }
    }
  }
  return matrix;
}

export function generateCleanMatrix() {
  const matrix = [];
  for (const fixtureManager of PACKAGE_MANAGERS) {
    for (const fixtureType of REPO_TYPES) {
      for (const interactive of BOOLEAN_OPTIONS) {
        for (const dry of BOOLEAN_OPTIONS) {
          matrix.push({
            fixtureManager,
            fixtureType,
            interactive,
            dry,
          });
        }
      }
    }
  }
  return matrix;
}

export function generateConvertLockMatrix() {
  const matrix = [];
  for (const fixtureManager of PACKAGE_MANAGERS) {
    for (const fixtureType of REPO_TYPES) {
      for (const toManager of PACKAGE_MANAGERS) {
        for (const interactive of BOOLEAN_OPTIONS) {
          for (const dry of BOOLEAN_OPTIONS) {
            matrix.push({
              fixtureManager,
              fixtureType,
              toManager,
              interactive,
              dry,
            });
          }
        }
      }
    }
  }
  return matrix;
}
