"use client";

import * as CollapsiblePrimitive from "@radix-ui/react-collapsible";
import { forwardRef } from "react";
import { cn } from "../cn";

const Collapsible = CollapsiblePrimitive.Root;

const CollapsibleTrigger = CollapsiblePrimitive.CollapsibleTrigger;

const CollapsibleContent = forwardRef<
  React.ElementRef<typeof CollapsiblePrimitive.CollapsibleContent>,
  React.ComponentPropsWithoutRef<typeof CollapsiblePrimitive.CollapsibleContent>
>(({ children, ...props }, ref) => {
  return (
    <CollapsiblePrimitive.CollapsibleContent
      ref={ref}
      {...props}
      className={cn(
        "overflow-hidden [--radix-collapsible-content-height:0px] data-[state=closed]:animate-fd-collapsible-up data-[state=open]:animate-fd-collapsible-down",
        props.className
      )}
    >
      {children}
    </CollapsiblePrimitive.CollapsibleContent>
  );
});

CollapsibleContent.displayName =
  CollapsiblePrimitive.CollapsibleContent.displayName;

export { Collapsible, CollapsibleTrigger, CollapsibleContent };
