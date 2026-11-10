export const actionClient = {
  schema(_s: unknown) {
    return {
      action<T>(fn: (args: { parsedInput: any }) => Promise<T>) {
        return (input: unknown) => fn({ parsedInput: input });
      },
    };
  },
};
