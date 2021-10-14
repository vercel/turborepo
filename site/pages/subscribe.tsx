import axios from 'axios'
import { ErrorMessage, Field, Form, Formik } from 'formik'
import Cookies from 'js-cookie'
import { NextSeo } from 'next-seo'
import { useRouter } from 'next/router'
import * as React from 'react'
import { Container } from '../components/container'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'

export interface SubscribeProps {}

export default function Subscribe(props: SubscribeProps) {
  const router = useRouter()
  return (
    <Layout showCta={false}>
      <NextSeo title="Turborepo" />
      <Container>
        <Header />
        <div className="pt-20 mx-auto ">
          <div
            className="max-w-md  mx-auto bg-white bg-opacity-5  text-white shadow-xl "
            style={{
              borderRadius: 24,
            }}
          >
            <div className="  p-6 shadow-sm" style={{ borderRadius: 24 }}>
              <Formik
                initialValues={{
                  email: '',
                  firstName: '',
                  lastName: '',
                }}
                onSubmit={async (values) => {
                  axios.post('/api/signup', values).then((res) => {
                    Cookies.set('ckId', res.data.id, {
                      expires: 365,
                    })
                    router
                      .push('/confirm', '/confirm')
                      .then(() => window.scrollTo(0, 0))
                  })
                }}
              >
                <Form className="space-y-4">
                  <div>
                    <div className="font-bold text-2xl text-center">
                      Subscribe for early access
                    </div>
                  </div>
                  <div>
                    <Field
                      name="firstName"
                      type="text"
                      placeholder="First Name"
                      className="rounded-md shadow-sm bg-white bg-opacity-5 text-white border-transparent focus:border-transparent focus:ring focus:ring-indigo-200 focus:ring-opacity-50 w-full"
                    />
                    <ErrorMessage
                      name="firstName"
                      component="div"
                      className="text-xs text-red-600"
                    />
                  </div>
                  <div>
                    <Field
                      name="lastName"
                      type="text"
                      placeholder="Last Name"
                      className="rounded-md shadow-sm bg-white bg-opacity-5 text-white border-transparent focus:border-transparent focus:ring focus:ring-indigo-200 focus:ring-opacity-50 w-full"
                    />
                    <ErrorMessage
                      name="lastName"
                      component="div"
                      className="text-xs text-red-600"
                    />
                  </div>
                  <div>
                    <Field
                      name="email"
                      type="email"
                      required={true}
                      placeholder="Email Address"
                      className="rounded-md shadow-sm bg-white bg-opacity-5 text-white border-transparent focus:border-transparent focus:ring focus:ring-indigo-200 focus:ring-opacity-50 w-full"
                    />

                    <ErrorMessage
                      name="email"
                      component="div"
                      className="text-xs text-red-600"
                    />
                  </div>
                  <button
                    type="submit"
                    className="inline-flex items-center justify-center px-5 py-3  text-base leading-6 font-medium rounded-md bg-gradient-to-r from-blue-500  to-red-500 text-blue-100  focus:outline-none focus:shadow-outline focus:border-blue-300 transition duration-150 ease-in-out w-full"
                  >
                    Subscribe
                  </button>
                </Form>
              </Formik>
            </div>
          </div>
        </div>
      </Container>
    </Layout>
  )
}

Subscribe.displayName = 'Subscribe'
