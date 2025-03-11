import { cn } from "../../utils/index";
import { ButtonElement, PrimitveButtonProps } from "@Configs/primitives";
import { Component, JSX, splitProps } from "solid-js";
import { Dynamic } from "solid-js/web";

// Button props
interface ButtonWrapperProps
  extends PrimitveButtonProps,
    JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: keyof JSX.IntrinsicElements | Component<any>;
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
