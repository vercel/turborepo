import Axios from 'axios'
import { ErrorMessage, Field, Form, Formik } from 'formik'
import * as React from 'react'
import * as Yup from 'yup'
import { formium } from '../lib/formium'

export function DemoForm() {
  const [success, setSuccess] = React.useState(false)
  if (success) {
    return (
      <div className=" max-w-md px-6 py-32   mx-auto">
        <div className="font-bold text-2xl relative inline-block bg-clip-text text-transparent bg-gradient-to-r from-blue-500 to-red-500">
          Thanks! We&apos;ll be in touch shortly.
        </div>
      </div>
    )
  }

  return (
    <div id="demo" className="mx-auto">
      <div
        className="max-w-md  mx-auto bg-white dark:bg-opacity-5  dark:text-white shadow-xl "
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
              teamSize: '',
              buildTools: '',
              whatHurts: '',
            }}
            validationSchema={Yup.object({
              firstName: Yup.string().required('Required'),
              lastName: Yup.string().required('Required'),
              email: Yup.string().email('Invalid Email').required('Required'),
              teamSize: Yup.string().required('Required'),
            })}
            onSubmit={async (values) => {
              await Axios.post('/api/signup', values)
              await formium.submitForm('turborepo-demo-request-form', values)
              setSuccess(true)
            }}
          >
            <Form className="space-y-4">
              <div>
                <div className="font-bold text-2xl text-center dark:text-white text-gray-900">
                  Request a Live Demo
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label htmlFor="firstName" className="sr-only">
                    First Name
                  </label>
                  <Field
                    name="firstName"
                    id="firstName"
                    type="text"
                    placeholder="First Name *"
                    className="rounded-md shadow-sm dark:bg-white dark:bg-opacity-5 dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0 duration-100 transition ease-in-out  w-full"
                  />
                  <ErrorMessage
                    name="firstName"
                    component="div"
                    className="text-xs text-red-600"
                  />
                </div>
                <div>
                  <label htmlFor="lastName" className="sr-only">
                    Last Name
                  </label>
                  <Field
                    name="lastName"
                    id="lastName"
                    type="text"
                    placeholder="Last Name *"
                    className="rounded-md shadow-sm dark:bg-white dark:bg-opacity-5 dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0 duration-100 transition ease-in-out  w-full"
                  />
                  <ErrorMessage
                    name="lastName"
                    component="div"
                    className="text-xs text-red-600"
                  />
                </div>
              </div>
              <div>
                <label htmlFor="email" className="sr-only">
                  Company Email
                </label>
                <Field
                  name="email"
                  id="email"
                  type="email"
                  placeholder="Company Email *"
                  className="rounded-md shadow-sm dark:bg-white dark:bg-opacity-5 dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0 duration-100 transition ease-in-out  w-full"
                />
                <ErrorMessage
                  name="email"
                  component="div"
                  className="text-xs text-red-600"
                />
              </div>
              <div>
                <label htmlFor="teamSize" className="sr-only">
                  Team Size
                </label>
                <Field
                  name="teamSize"
                  id="teamSize"
                  component="select"
                  placeholder="How big is your engineering team? "
                  className="rounded-md shadow-sm dark:bg-[#191a1b] dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0 duration-100 transition ease-in-out  w-full"
                >
                  <option value="">
                    How big is your company or engineering team?
                  </option>
                  <option>1-5</option>
                  <option>5-10</option>
                  <option>10-25</option>
                  <option>25-50</option>
                  <option>50-125</option>
                  <option>125-250</option>
                  <option>250-750</option>
                  <option>750-1500</option>
                  <option>1500+</option>
                </Field>

                <ErrorMessage
                  name="teamSize"
                  component="div"
                  className="text-xs text-red-600"
                />
              </div>
              <div>
                <label htmlFor="buildTools" className="sr-only">
                  What build tools do you currently use?
                </label>
                <Field
                  name="buildTools"
                  id="buildTools"
                  type="text"
                  placeholder="What build tools do you currently use?"
                  className="rounded-md shadow-sm dark:bg-white dark:bg-opacity-5 dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0 duration-100 transition ease-in-out  w-full"
                />

                <ErrorMessage
                  name="text"
                  component="div"
                  className="text-xs text-red-600"
                />
              </div>
              <div>
                <label htmlFor="whatHurts" className="sr-only">
                  What&apos;s the most challenging part of your monorepo?
                  What&apos;s keeping you from shipping faster?
                </label>
                <Field
                  name="whatHurts"
                  id="whatHurts"
                  component="textarea"
                  rows={4}
                  placeholder="What's the most challenging part of your monorepo? What's keeping you from shipping faster?"
                  className="rounded-md shadow-sm dark:bg-white dark:bg-opacity-5 dark:text-white text-gray-900 dark:border-transparent border border-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 dark:focus:ring-white dark:focus:border-white dark:focus:ring-0  w-full"
                />

                <ErrorMessage
                  name="text"
                  component="div"
                  className="text-xs text-red-600"
                />
              </div>
              <button
                type="submit"
                style={{
                  borderRadius: '12px',
                }}
                className="inline-flex  items-center justify-center px-5 py-3  text-base leading-6 font-medium rounded-md bg-gradient-to-r from-blue-500  to-red-500 text-white  focus:outline-none focus:shadow-outline focus:border-blue-300 transition duration-150 ease-in-out w-full"
              >
                Request Demo
              </button>
              <div className="text-center text-xs text-gray-700">
                By submitting your info, you agree to opt-in to emails from
                Turborepo.
              </div>
            </Form>
          </Formik>
        </div>
      </div>
    </div>
  )
}

DemoForm.displayName = 'DemoForm'
