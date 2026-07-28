import React from "react";

interface HeaderProps {
  title: string;
}

export const Header: React.FC<HeaderProps> = ({ title }) => {
  return (
    <header id="header">
      <h1>{title}</h1>
    </header>
  );
};
