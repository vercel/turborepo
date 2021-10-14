import { NextSeo } from 'next-seo'
import * as React from 'react'
import { Container } from '../components/container'
import { DemoForm } from '../components/DemoForm'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'

const Index = () => {
  return (
    <Layout showCta={false}>
      <NextSeo
        title="Turborepo"
        titleTemplate="Turborepo"
        description="The blazing fast monorepo build system"
      />
      <Container>
        <Header />
        <div>
          <div className=" py-16 md:py-24 ">
            <h1 className="pb-6 lg:pb-12 text-5xl xl:text-9xl lg:text-center  max-w-3xl lg:max-w-none md:text-7xl lg:text-8xl  font-black text-gray-800 tracking-tight leading-snug sm:leading-snug md:leading-tight">
              <span className="dark:text-white">Monorepos that</span>
              <span className="relative inline-block bg-clip-text text-transparent bg-gradient-to-r from-blue-500 to-red-500">
                make ship happen.
              </span>
            </h1>
            <div className="prose md:prose-xl lg:prose-2xl prose-blue-600 mx-auto lg:mt-2 max-w-3xl dark:text-gray-200">
              <p>
                Monorepos are incredible for productivity, but the tooling can
                be a nightmare. Between Yarn, TypeScript, React, Babel, Webpack,
                frameworks like Next.js and Gatsby, Jest, Prettier, Rollup,
                ESLint, environment variables, changelogs, and getting VSCode to
                understand it all...there is just a lot of stuff to do (and
                things to mess up). Nothing &ldquo;just works.&rdquo; It&apos;s
                become completely normal to waste entire days on just
                tooling—tweaking configs, writing one-off scripts, and stitching
                stuff together.
              </p>
              <p>We need something else.</p>
              <p>
                A fresh take on the whole setup. Designed to glue everything
                together. A toolchain that works for you and not against you.
                With sensible defaults, but even better escape hatches. Built
                with the same techniques used by the big guys, but in a way that
                doesn&apos;t require PhD to learn or a staff to maintain.
              </p>
              <p>
                With Turborepo, we&apos;re doing just that. We&apos;re
                abstracting the complex configuration needed for most monorepos
                into a single cohesive build system—giving you a world class
                development experience without the maintenance burden.
              </p>
              <p>
                🖖,
                <br />
                <a
                  href="https://twitter.com/jaredpalmer"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Jared P.
                </a>
              </p>
            </div>
            <div className="pt-24">
              <DemoForm />
            </div>
          </div>
        </div>
      </Container>
    </Layout>
  )
}

export default Index
