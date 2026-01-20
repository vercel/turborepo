"use client";

import { MenuIcon } from "lucide-react";
import { useSidebarContext } from "@/hooks/geistdocs/use-sidebar";
import { cn } from "@/lib/utils";
import { Button } from "../ui/button";

type MobileMenuProps = {
  className?: string;
};

export const MobileMenu = ({ className }: MobileMenuProps) => {
  const { isOpen, setIsOpen } = useSidebarContext();

  return (
    <Button
      className={cn(className)}
      onClick={() => setIsOpen(!isOpen)}
      size="icon-sm"
      variant="ghost"
    >
      <MenuIcon className="size-4" />
    </Button>
  );
};
