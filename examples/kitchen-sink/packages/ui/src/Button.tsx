export const Button = ({ children, ...other }) => {
  return (
    <button target="_blank" rel="noreferrer" {...other}>
      {children}
    </button>
  );
};
