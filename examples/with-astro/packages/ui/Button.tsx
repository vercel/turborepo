export const Button = () => {
  return (
    <button
      onClick={() => {
        throw Error("astro");
      }}
    >
      Boop
    </button>
  );
};
