import React from "react";

function Hero() {
  return (
    <>
      <h1 className="text-center text-6xl font-extrabold tracking-tighter leading-[1.1] sm:text-7xl lg:text-8xl xl:text-8xl -mx-24 pt-16">
        Monorepos that
        <br className="hidden lg:block" />
        <span className="inline-block bg-clip-text text-transparent bg-gradient-to-r from-[#83FFD2] to-[#35ACDF] ">
          make ship happen.
        </span>
      </h1>
      <p className="max-w-lg mx-auto mt-6 text-xl font-medium leading-tight text-center text-gray-400 sm:max-w-4xl sm:text-2xl md:text-3xl lg:text-4xl">
        Turborepo is a high-performance build system for modern codebases.
      </p>
    </>
  );
}

export default Hero;
