const positiveInteger = (value) => Number.isSafeInteger(value) && value > 0;
const nonemptyString = (value) => typeof value === "string" && value.length > 0;
const object = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);

// Only REST pull.stack metadata can establish native stack eligibility.
// Return a scalar snapshot to pin membership and root base across CI polls.
// An absent/null stack is an ordinary PR; malformed metadata is never ignored,
// even when the immediate base is already the default branch.
export function reviewBaseIdentity(pull) {
  const defaultBranch = pull?.base?.repo?.default_branch;
  if (!nonemptyString(defaultBranch) || !nonemptyString(pull?.base?.ref)) {
    return undefined;
  }
  const stack = pull.stack;
  if (stack == null) {
    return pull.base.ref === defaultBranch
      ? JSON.stringify([defaultBranch, null])
      : undefined;
  }
  if (
    !object(stack) ||
    !object(stack.base) ||
    stack.base.ref !== defaultBranch ||
    !nonemptyString(stack.base.sha) ||
    !positiveInteger(stack.id) ||
    !positiveInteger(stack.number) ||
    !positiveInteger(stack.position) ||
    !positiveInteger(stack.size) ||
    stack.position > stack.size
  ) {
    return undefined;
  }
  return JSON.stringify([
    defaultBranch,
    stack.id,
    stack.number,
    stack.position,
    stack.size,
    stack.base.ref,
    stack.base.sha,
  ]);
}
