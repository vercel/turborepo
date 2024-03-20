<script setup lang="ts">
import Card from "./card.vue";
import Gradient from "./gradient.vue";

const LINKS = [
  {
    title: "Docs",
    href: "https://turbo.build/repo/docs",
    description: "Find in-depth information about Turborepo features and API.",
  },
  {
    title: "Learn",
    href: "https://turbo.build/repo/docs/handbook",
    description: "Learn more about monorepos with our handbook.",
  },
  {
    title: "Templates",
    href: "https://turbo.build/repo/docs/getting-started/from-example",
    description: "Choose from over 15 examples and deploy with a single click.",
  },
  {
    title: "Deploy",
    href: "https://vercel.com/new",
    description:
      "Instantly deploy your Turborepo to a shareable URL with Vercel.",
  },
];

const props = defineProps<{
  appName: string;
}>();
</script>

<template>
  <main class="main">
    <div class="description">
      <p>
        examples/vue-nuxt&nbsp;
        <code class="code">{{ props.appName }}</code>
      </p>

      <div>
        <a
          href="https://vercel.com?utm_source=create-turbo&utm_medium=basic&utm_campaign=create-turbo"
          rel="noopener noreferrer"
          target="_blank"
        >
          By{{ " " }}
          <img
            alt="Vercel Logo"
            class="vercelLogo"
            height="24"
            priority
            src="./assets/vercel.svg"
            width="100"
          />
        </a>
      </div>
    </div>

    <div class="hero">
      <div class="heroContent">
        <div class="logos">
          <div class="circles">
            <img
              alt="Turborepo"
              height="614"
              src="./assets/circles.svg"
              width="614"
            />
          </div>

          <div class="logoGradientContainer">
            <Gradient class="logoGradient" :conic="true" :small="true" />
          </div>

          <div class="logo">
            <img
              alt=""
              height="120"
              priority
              src="./assets/turborepo.svg"
              width="120"
            />
          </div>
        </div>
        <Gradient class="backgroundGradient" conic />
        <div class="turborepoWordmarkContainer">
          <svg
            class="turborepoWordmark"
            viewBox="0 0 506 50"
            width="200"
            xmlns="http://www.w3.org/2000/svg"
          >
            <title>Turborepo logo</title>
            <path
              d="M53.7187 12.0038V1.05332H0.945312V12.0038H20.8673V48.4175H33.7968V12.0038H53.7187Z"
            />
            <path
              d="M83.5362 49.1431C99.764 49.1431 108.67 40.8972 108.67 27.3081V1.05332H95.7401V26.0547C95.7401 33.6409 91.7821 37.9287 83.5362 37.9287C75.2904 37.9287 71.3324 33.6409 71.3324 26.0547V1.05332H58.4029V27.3081C58.4029 40.8972 67.3084 49.1431 83.5362 49.1431Z"
            />
            <path
              d="M128.462 32.7174H141.325L151.484 48.4175H166.327L154.848 31.3321C161.313 29.0232 165.271 23.8778 165.271 16.8853C165.271 6.72646 157.685 1.05332 146.141 1.05332H115.532V48.4175H128.462V32.7174ZM128.462 22.4925V11.8719H145.481C150.033 11.8719 152.54 13.8509 152.54 17.2152C152.54 20.3816 150.033 22.4925 145.481 22.4925H128.462Z"
            />
            <path
              d="M171.287 48.4175H205.128C215.683 48.4175 221.752 43.404 221.752 35.0262C221.752 29.419 218.189 25.593 213.967 23.8778C216.87 22.4925 220.432 19.1942 220.432 13.9828C220.432 5.60502 214.495 1.05332 204.006 1.05332H171.287V48.4175ZM183.689 19.59V11.542H202.687C206.249 11.542 208.228 12.9273 208.228 15.566C208.228 18.2047 206.249 19.59 202.687 19.59H183.689ZM183.689 29.2871H203.875C207.371 29.2871 209.284 31.0022 209.284 33.5749C209.284 36.1476 207.371 37.8628 203.875 37.8628H183.689V29.2871Z"
            />
            <path
              d="M253.364 0.261719C236.806 0.261719 224.866 10.6185 224.866 24.7354C224.866 38.8523 236.806 49.2091 253.364 49.2091C269.922 49.2091 281.796 38.8523 281.796 24.7354C281.796 10.6185 269.922 0.261719 253.364 0.261719ZM253.364 11.4761C262.072 11.4761 268.602 16.6215 268.602 24.7354C268.602 32.8493 262.072 37.9947 253.364 37.9947C244.656 37.9947 238.126 32.8493 238.126 24.7354C238.126 16.6215 244.656 11.4761 253.364 11.4761Z"
            />
            <path
              d="M300.429 32.7174H313.292L323.451 48.4175H338.294L326.815 31.3321C333.28 29.0232 337.238 23.8778 337.238 16.8853C337.238 6.72646 329.652 1.05332 318.108 1.05332H287.499V48.4175H300.429V32.7174ZM300.429 22.4925V11.8719H317.448C322 11.8719 324.507 13.8509 324.507 17.2152C324.507 20.3816 322 22.4925 317.448 22.4925H300.429Z"
            />
            <path
              d="M343.254 1.05332V48.4175H389.299V37.467H355.92V29.7489H385.539V19.0622H355.92V12.0038H389.299V1.05332H343.254Z"
            />
            <path
              d="M408.46 33.3111H425.677C437.221 33.3111 444.807 27.7699 444.807 17.2152C444.807 6.59453 437.221 1.05332 425.677 1.05332H395.53V48.4175H408.46V33.3111ZM408.46 22.5585V11.8719H424.951C429.569 11.8719 432.076 13.8509 432.076 17.2152C432.076 20.5135 429.569 22.5585 424.951 22.5585H408.46Z"
            />
            <path
              d="M476.899 0.261719C460.341 0.261719 448.401 10.6185 448.401 24.7354C448.401 38.8523 460.341 49.2091 476.899 49.2091C493.456 49.2091 505.33 38.8523 505.33 24.7354C505.33 10.6185 493.456 0.261719 476.899 0.261719ZM476.899 11.4761C485.606 11.4761 492.137 16.6215 492.137 24.7354C492.137 32.8493 485.606 37.9947 476.899 37.9947C468.191 37.9947 461.66 32.8493 461.66 24.7354C461.66 16.6215 468.191 11.4761 476.899 11.4761Z"
            />
          </svg>
        </div>
      </div>
    </div>

    <div class="grid">
      <Card
        v-for="{ title, href, description } in LINKS"
        class="card"
        :href="href"
        :key="title"
        :title="title"
      >
        {{ description }}
      </Card>
    </div>
  </main>
</template>

<style>
@import url("https://fonts.googleapis.com/css2?family=Inter:wght@100;200;300;400;500;600;700;800;900&display=swap");

:root {
  --max-width: 1100px;
  --border-radius: 12px;
  --font-mono: ui-monospace, Menlo, Monaco, "Cascadia Mono", "Segoe UI Mono",
    "Roboto Mono", "Oxygen Mono", "Ubuntu Monospace", "Source Code Pro",
    "Fira Mono", "Droid Sans Mono", "Courier New", monospace;

  --foreground-rgb: 255, 255, 255;
  --background-start-rgb: 0, 0, 0;
  --background-end-rgb: 0, 0, 0;

  --callout-rgb: 20, 20, 20;
  --callout-border-rgb: 108, 108, 108;
  --card-rgb: 100, 100, 100;
  --card-border-rgb: 200, 200, 200;

  --glow-conic: conic-gradient(
    from 180deg at 50% 50%,
    #2a8af6 0deg,
    #a853ba 180deg,
    #e92a67 360deg
  );
}

* {
  box-sizing: border-box;
  padding: 0;
  margin: 0;
}

html,
body {
  max-width: 100vw;
  overflow-x: hidden;
  font-family: "Inter", sans-serif;
}

body {
  color: rgb(var(--foreground-rgb));
  background: linear-gradient(
      to bottom,
      transparent,
      rgb(var(--background-end-rgb))
    )
    rgb(var(--background-start-rgb));
}

a {
  color: inherit;
  text-decoration: none;
}

.main {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  padding: 6rem;
  min-height: 100vh;
}

.vercelLogo {
  filter: invert(1);
}

.description {
  display: inherit;
  justify-content: inherit;
  align-items: inherit;
  font-size: 0.85rem;
  max-width: var(--max-width);
  width: 100%;
  z-index: 2;
  font-family: var(--font-mono);
}

.description a {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 0.5rem;
}

.description p {
  position: relative;
  margin: 0;
  padding: 1rem;
  background-color: rgba(var(--callout-rgb), 0.5);
  border: 1px solid rgba(var(--callout-border-rgb), 0.3);
  border-radius: var(--border-radius);
}

.code {
  font-weight: 700;
  font-family: var(--font-mono);
}

.hero {
  display: flex;
  position: relative;
  place-items: center;
}

.heroContent {
  display: flex;
  position: relative;
  z-index: 0;
  padding-bottom: 4rem;
  flex-direction: column;
  gap: 2rem;
  justify-content: space-between;
  align-items: center;
  width: auto;
  font-family: system-ui, "Segoe UI", Roboto, "Helvetica Neue", Arial,
    "Noto Sans", sans-serif, "Apple Color Emoji", "Segoe UI Emoji",
    "Segoe UI Symbol", "Noto Color Emoji";
  padding-top: 48px;

  @media (min-width: 768px) {
    padding-top: 4rem;
    padding-bottom: 6rem;
  }
  @media (min-width: 1024px) {
    padding-top: 5rem;
    padding-bottom: 8rem;
  }
}

.logos {
  display: flex;
  z-index: 50;
  justify-content: center;
  align-items: center;
  width: 100%;
}

.grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(25%, auto));
  max-width: 100%;
  width: var(--max-width);
}

.card {
  padding: 1rem 1.2rem;
  border-radius: var(--border-radius);
  background: rgba(var(--card-rgb), 0);
  border: 1px solid rgba(var(--card-border-rgb), 0);
  transition:
    background 200ms,
    border 200ms;
}

.card span {
  display: inline-block;
  transition: transform 200ms;
}

.card h2 {
  font-weight: 600;
  margin-bottom: 0.7rem;
}

.card p {
  margin: 0;
  opacity: 0.6;
  font-size: 0.9rem;
  line-height: 1.5;
  max-width: 30ch;
}

@media (prefers-reduced-motion) {
  .card:hover span {
    transform: none;
  }
}

/* Mobile */
@media (max-width: 700px) {
  .content {
    padding: 4rem;
  }

  .grid {
    grid-template-columns: 1fr;
    margin-bottom: 120px;
    max-width: 320px;
    text-align: center;
  }

  .card {
    padding: 1rem 2.5rem;
  }

  .card h2 {
    margin-bottom: 0.5rem;
  }

  .center {
    padding: 8rem 0 6rem;
  }

  .center::before {
    transform: none;
    height: 300px;
  }

  .description {
    font-size: 0.8rem;
  }

  .description a {
    padding: 1rem;
  }

  .description p,
  .description div {
    display: flex;
    justify-content: center;
    position: fixed;
    width: 100%;
  }

  .description p {
    align-items: center;
    inset: 0 0 auto;
    padding: 2rem 1rem 1.4rem;
    border-radius: 0;
    border: none;
    border-bottom: 1px solid rgba(var(--callout-border-rgb), 0.25);
    background: linear-gradient(
      to bottom,
      rgba(var(--background-start-rgb), 1),
      rgba(var(--callout-rgb), 0.5)
    );
    background-clip: padding-box;
    backdrop-filter: blur(24px);
  }

  .description div {
    align-items: flex-end;
    pointer-events: none;
    inset: auto 0 0;
    padding: 2rem;
    height: 200px;
    background: linear-gradient(
      to bottom,
      transparent 0%,
      rgb(var(--background-end-rgb)) 40%
    );
    z-index: 1;
  }
}

/* Enable hover only on non-touch devices */
@media (hover: hover) and (pointer: fine) {
  .card:hover {
    background: rgba(var(--card-rgb), 0.1);
    border: 1px solid rgba(var(--card-border-rgb), 0.15);
  }

  .card:hover span {
    transform: translateX(4px);
  }
}

.circles {
  position: absolute;
  min-width: 614px;
  min-height: 614px;
}

.logo {
  z-index: 50;
  width: 120px;
  height: 120px;
}

.logoGradientContainer {
  display: flex;
  position: absolute;
  z-index: 50;
  justify-content: center;
  align-items: center;
  width: 16rem;
  height: 16rem;
}

.turborepoWordmarkContainer {
  display: flex;
  z-index: 50;
  padding-left: 1.5rem;
  padding-right: 1.5rem;
  flex-direction: column;
  gap: 1.25rem;
  justify-content: center;
  align-items: center;
  text-align: center;

  @media (min-width: 1024px) {
    gap: 1.5rem;
  }
}

.turborepoWordmark {
  width: 160px;
  fill: white;

  @media (min-width: 768px) {
    width: 200px;
  }
}

.code {
  font-family: Menlo, Monaco, Consolas, "Liberation Mono", "Courier New",
    monospace;
  font-weight: 700;
}

/* Tablet and Smaller Desktop */
@media (min-width: 701px) and (max-width: 1120px) {
  .grid {
    grid-template-columns: repeat(2, 50%);
  }
}

/* Gradients */
.gradient {
  position: absolute;
  mix-blend-mode: normal;
  will-change: filter;
}

.gradientSmall {
  filter: blur(32px);
}

.gradientLarge {
  filter: blur(75px);
}

.glowConic {
  background-image: var(--glow-conic);
}

.logoGradient {
  opacity: 0.9;
  width: 120px;
  height: 120px;
}

.backgroundGradient {
  top: -500px;
  width: 1000px;
  height: 1000px;
  opacity: 0.15;
}
</style>
