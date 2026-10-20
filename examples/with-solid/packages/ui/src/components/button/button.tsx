import { Component, JSX, splitProps, ValidComponent } from "solid-js";
import { Dynamic } from "solid-js/web";
import { ButtonElement, PrimitiveButtonProps } from "../../config/primitives";
import { cn } from "../../utils/index";

// Button props
interface ButtonWrapperProps
  extends PrimitiveButtonProps,
    JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: ValidComponent;
  children?: JSX.Element;
  ref?: (el: ButtonElement) => void;
  class?: string;
}

// Button Component
const Button: Component<ButtonWrapperProps> = (props) => {
  // separate our special props from the rest
  const [local, others] = splitProps(props, [
    "asChild",
    "ref",
    "children",
    "class",
  ]);

  return (
    <Dynamic
      component={local.asChild || "button"}
      ref={local.ref}
      class={cn(
        "flex items-center justify-center gap-3  outline-none cursor-pointer p-4 rounded-sm text-[1rem] w-full mx-auto",
        local?.class,
      )}
      {...others}
    >
      {local.children}
    </Dynamic>
  );
};

// export
export { Button };
