interface ButtonProps {
  /**
   * What background color to use
   */
  backgroundColor: "red" | "blue";
  /**
   * How large should the button be?
   */
  size?: "small" | "medium" | "large";
  /**
   * Button contents
   */
  label: string;
  /**
   * Optional click handler
   */
  onClick?: () => void;
}

/**
 * Primary UI component for user interaction
 */
export const Button = ({
  size = "medium",
  backgroundColor,
  label,
  ...props
}: ButtonProps) => {
  return (
    <button
      type="button"
      style={{
        backgroundColor:
          backgroundColor === "red"
            ? "rgba(255,30,86,1)"
            : "rgba(50,134,241,1)",
        color: "white",
        padding: "8px",
        borderRadius: "8px",
        border: "none",
        cursor: "pointer",
      }}
      {...props}
    >
      {label}
    </button>
  );
};
