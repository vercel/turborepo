import { NextSeo } from 'next-seo'
import * as React from 'react'
import { Container } from '../components/container'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { DemoForm } from '../components/DemoForm'

const RequestDemo = () => {
  return (
    <Layout showCta={false}>
      <NextSeo
        title="Request a Live Demo"
        description="Request a customized live demo of Turborepo"
      />
      <Container>
        <Header />
        <div className="mx-auto container">
          <div className="lg:mx-auto lg:max-w-7xl lg:px-8 lg:grid lg:grid-cols-2 lg:grid-flow-col-dense lg:gap-24">
            <div className="px-4 max-w-xl mx-auto sm:px-6 lg:py-16 lg:max-w-none lg:mx-0 lg:px-0">
              <div>
                <div className="mt-6">
                  <h2 className="text-3xl font-extrabold tracking-tight text-white">
                    Give us 30 days, and we&apos;ll cut your build time in half.
                  </h2>
                  <p className="mt-4 text-lg text-gray-500">
                    Experience the FREE Turborepo Proof of Value 30-Day Trial.
                  </p>
                  <p className="mt-4 text-lg text-gray-500">
                    The Turborepo Trial will provide an eye-opening and
                    energizing experience and exposure to the next-generation of
                    build engineering technology. You can expect to:
                  </p>
                  <div className="mt-6">
                    <a
                      href="#"
                      className="inline-flex px-4 py-2 border border-transparent text-base font-medium rounded-md shadow-sm text-white bg-gradient-to-r from-purple-600 to-indigo-600 betterhover:hover:from-purple-700 betterhover:hover:to-indigo-700"
                    >
                      Get started
                    </a>
                  </div>
                </div>
              </div>
              <div className="mt-8 border-t border-gray-200 pt-6">
                <blockquote>
                  <div>
                    <p className="text-base text-gray-500">
                      “Cras velit quis eros eget rhoncus lacus ultrices sed
                      diam. Sit orci risus aenean curabitur donec aliquet. Mi
                      venenatis in euismod ut.”
                    </p>
                  </div>
                  <footer className="mt-3">
                    <div className="flex items-center space-x-3">
                      <div className="flex-shrink-0">
                        <img
                          className="h-6 w-6 rounded-full"
                          src="https://images.unsplash.com/photo-1509783236416-c9ad59bae472?ixlib=rb-=eyJhcHBfaWQiOjEyMDd9&auto=format&fit=facearea&facepad=8&w=1024&h=1024&q=80"
                          alt=""
                        />
                      </div>
                      <div className="text-base font-medium text-gray-700">
                        Marcia Hill, Digital Marketing Manager
                      </div>
                    </div>
                  </footer>
                </blockquote>
              </div>
            </div>
            <div className="mt-12 sm:mt-16 lg:mt-0">
              <div className="max-w-md mx-auto shadow-xl rounded-lg bg-white bg-opacity-5">
                <div className=" rounded-lg p-6 shadow-sm">
                  <DemoForm />
                </div>
              </div>
            </div>
          </div>
        </div>
      </Container>
    </Layout>
  )
}

export default RequestDemo
