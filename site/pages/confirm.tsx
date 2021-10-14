/* eslint-disable react/no-unescaped-entities */
import { VisuallyHidden } from '@react-aria/visually-hidden'
import axios from 'axios'
import Head from 'next/head'
import * as React from 'react'
import { Container } from '../components/container'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { Radio, RadioGroup } from '../components/RadioGroup'
import { useCkViewer } from '../components/useCkViewer'

const Index = () => {
  const { data, mutate } = useCkViewer()
  return (
    <Layout showCta={false}>
      <Head>
        <title>Confirm</title>
        <meta name="robots" content="noindex" />
      </Head>
      <Container>
        <Header />
        <div className="mx-auto container">
          <div className="pt-20 mx-auto ">
            <div className="max-w-md mx-auto shadow-xl rounded-lg bg-white bg-opacity-5">
              <div className=" rounded-lg p-6 shadow-sm">
                {data && data?.fields?.job_title ? (
                  <div className="space-y-4 mx-auto text-white">
                    <h2 className="text-xl font-bold">
                      Thanks so much! There's one last step.
                    </h2>
                    <p>
                      <strong>Please confirm your email.</strong> Please check
                      your inbox for an email that just got sent. You'll need to
                      click the confirmation link to receive any further emails.
                    </p>{' '}
                    <p>
                      If you don't see the email after a few minutes, you might
                      check your spam folder or other filters and add{' '}
                      <code className="text-sm bg-white bg-opacity-5 rounded-sm p-1">
                        hello@turborepo.com
                      </code>{' '}
                      to your contacts.
                    </p>
                    <p>
                      Thanks,
                      <br />
                      Jared
                    </p>
                  </div>
                ) : (
                  <div className="space-y-4 mx-auto text-white">
                    <div className="font-semibold text-2xl text-center leading-tight">
                      How would you describe yourself?
                    </div>
                    <RadioGroup
                      label={
                        <VisuallyHidden>
                          <span className="mb-2 font-medium">Choose one:</span>
                        </VisuallyHidden>
                      }
                      onChange={(job_title) => {
                        axios
                          .put(`/api/user/${data.id}`, {
                            ...data,
                            fields: {
                              ...data.fields,
                              job_title,
                            },
                          })
                          .then((res) => {
                            mutate(res.data)
                          })
                      }}
                    >
                      <Radio value="manager">Engineering Manager</Radio>
                      <Radio value="senior">Senior Developer</Radio>
                      <Radio value="junior">Junior Developer</Radio>
                      <Radio value="novice">Novice Developer</Radio>
                      <Radio value="none">Choose not to say</Radio>
                    </RadioGroup>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </Container>
    </Layout>
  )
}

export default Index
