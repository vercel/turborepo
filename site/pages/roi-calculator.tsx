import { useLocale } from '@react-aria/i18n'
import { NextSeo } from 'next-seo'
import { useRouter } from 'next/router'
import qs from 'querystring'
import * as React from 'react'
import { Container } from '../components/container'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { NumberInput } from '../components/NumberInput'
import { Slider } from '../components/SliderInput'
import { Sticky } from '../components/Sticky'
import { useMediaQuery } from '../components/useMediaQuery'
import { copyToClipboard } from '../lib/copyToClipbord'

const Noop = (props: any) => <>{props.children}</>

// 5 days / 40 hours
const minutesPerWorkWeek = 2400

const H2 = (props: any) => (
  <h2
    className="dark:text-white text-2xl lg:text-3xl font-extrabold text-center"
    {...props}
  />
)

const H3 = (props: any) => (
  <h3
    className="dark:text-white text-gray-900 text-xl lg:text-2xl font-extrabold text-center"
    {...props}
  />
)

const Card = (props: any) => (
  <div
    className="max-w-lg dark:border-transparent border lg:max-w-5xl space-y-4 w-full mx-auto bg-white bg-opacity-5 p-6 lg:p-8 xl:p-10 text-white shadow-xl "
    style={{
      borderRadius: 24,
    }}
    {...props}
  />
)

export default function Roi() {
  const [costPerDeveloper, setCostPerDeveloper] = React.useState(150000)
  const [devCount, setDevCount] = React.useState(10)
  const [workWeeks, setWorkWeeks] = React.useState(48)
  const [devBuildsPerDay, setDevBuildsPerDay] = React.useState(5)
  const [ciBuildsPerWeek, setCiBuildsPerWeek] = React.useState(500)
  const [avgLocalBuildMinutes, setAvgLocalBuildMinutes] = React.useState(10)
  const [avgCiBuildMinutes, setAvgCiBuildMinutes] = React.useState(10)
  const [percentLocalWaitTimeReduced, setPercentLocalWaitTimeReduced] =
    React.useState(0.5)
  const [percentCiWaitTimeReduced, setPercentCiWaitTimeReduced] =
    React.useState(0.5)
  const [percentCiDevWaitTimeReduced, setPercentCiDevWaitTimeReduced] =
    React.useState(0.2)
  const [didCopy, setDidCopy] = React.useState(false)
  React.useEffect(() => {
    const query: any = qs.parse(window.location.search.substr(1))
    if (query.costPerDeveloper) {
      setCostPerDeveloper(query.costPerDeveloper)
      setDevCount(query.devCount)
      setWorkWeeks(query.workWeeks)
      setDevBuildsPerDay(query.devBuildsPerDay)
      setCiBuildsPerWeek(query.ciBuildsPerWeek)
      setAvgLocalBuildMinutes(query.avgLocalBuildMinutes)
      setAvgCiBuildMinutes(query.avgCiBuildMinutes)
      setPercentLocalWaitTimeReduced(query.percentLocalWaitTimeReduced)
      setPercentCiWaitTimeReduced(query.percentCiWaitTimeReduced)
      setPercentCiDevWaitTimeReduced(query.percentCiDevWaitTimeReduced)
    }
  }, [])
  const router = useRouter()
  const getUrl = () =>
    '/roi-calculator' +
    '?' +
    qs.stringify({
      costPerDeveloper,
      devCount,
      workWeeks,
      devBuildsPerDay,
      ciBuildsPerWeek,
      avgLocalBuildMinutes,
      avgCiBuildMinutes,
      percentLocalWaitTimeReduced,
      percentCiWaitTimeReduced,
      percentCiDevWaitTimeReduced,
    })
  const shareLink = async () => {
    await router.replace(getUrl())
    copyToClipboard(window.location.href)
    setDidCopy(true)
  }
  const { locale } = useLocale()

  const savedLocalDevMinutesPerBuild =
    avgLocalBuildMinutes * percentLocalWaitTimeReduced
  const devBuildsPerWeek = devBuildsPerDay * 5
  const savedLocalDevHoursPerWeekPerDev =
    (savedLocalDevMinutesPerBuild / 60) * devBuildsPerWeek
  const savedLocalDevHoursPerWeek = savedLocalDevHoursPerWeekPerDev * devCount
  const annualSavedLocalDevHours =
    savedLocalDevHoursPerWeekPerDev * devCount * workWeeks
  const annualLocalDevCostRecapture =
    (savedLocalDevMinutesPerBuild *
      devBuildsPerWeek *
      devCount *
      costPerDeveloper) /
    minutesPerWorkWeek

  const savedCiMinutesPerBuild = avgCiBuildMinutes * percentCiWaitTimeReduced

  const savedCiHoursPerBuild = savedCiMinutesPerBuild / 60

  const savedCiHoursPerWeek =
    savedCiHoursPerBuild * ciBuildsPerWeek * percentCiDevWaitTimeReduced
  const savedCiHoursPerWeekPerDev =
    (savedCiHoursPerBuild * ciBuildsPerWeek * percentCiDevWaitTimeReduced) /
    devCount
  const annualCiCostCapture =
    ((avgCiBuildMinutes *
      (1 - percentCiWaitTimeReduced) *
      ciBuildsPerWeek *
      costPerDeveloper) /
      minutesPerWorkWeek) *
    percentCiDevWaitTimeReduced
  const totalCostRecapture = annualCiCostCapture + annualLocalDevCostRecapture
  const isMobile = useMediaQuery('(max-width: 1000px)')
  const Wrapper = isMobile ? Noop : Sticky
  return (
    <Layout showCta={true}>
      <NextSeo
        title="ROI Calculator"
        description="How much can Turborepo boost developer productivity?"
        openGraph={{
          url: `https://turborepo.com/roi-calculator`,
          title: 'Developer Productivity ROI Calculator',
          description: 'How much can Turborepo boost developer productivity?',
          images: [
            {
              url: 'https://turborepo.com/roi-calculator.png',
            },
          ],
        }}
      />
      <>
        <Container>
          <Header />
        </Container>
        <Wrapper>
          <div className="dark:bg-black bg-white">
            <div>
              <div className="py-8  mx-auto">
                <Container>
                  <h1 className="text-2xl  block dark:text-white tracking-tight leading-snug sm:leading-snug md:leading-tight font-extrabold  sm:text-4xl sm:text-center">
                    How much can Turborepo{' '}
                    <span className="relative inline-block bg-clip-text text-transparent bg-gradient-to-r from-blue-500 to-red-500">
                      boost your productivity?
                    </span>
                  </h1>

                  <Results
                    locale={locale}
                    totalCostRecapture={totalCostRecapture}
                    annualSavedLocalDevHours={annualSavedLocalDevHours}
                    savedLocalDevHoursPerWeekPerDev={
                      savedLocalDevHoursPerWeekPerDev
                    }
                    savedCiHoursPerWeekPerDev={savedCiHoursPerWeekPerDev}
                  />
                </Container>
              </div>
            </div>
          </div>
        </Wrapper>
        <Container>
          <div className="py-16  space-y-24">
            <div className="space-y-12">
              <div className="max-w-4xl mx-auto sm:text-center">
                <H2>Size Your Team and Build</H2>
                <p className="mt-3 text-xl text-gray-500 sm:mt-4 sm:text-center">
                  This calculator is a model that we have developed by working
                  with product teams to quantify the impact of build times on
                  developer productivity. Set the blue input fields to drive the
                  results above.
                </p>
              </div>
              <div className="grid gap-y-12 lg:gap-5 grid-cols-1 lg:grid-cols-2 lg:grid-flow-row max-w-5xl mx-auto">
                <Card>
                  <Slider
                    label="How many developers are on your team?"
                    value={[devCount]}
                    maxValue={150}
                    minValue={0}
                    step={5}
                    onChange={(value) => setDevCount(value[0])}
                  />
                  <Slider
                    label="Cost per developer"
                    value={[costPerDeveloper]}
                    maxValue={300000}
                    step={1000}
                    minValue={0}
                    formatOptions={{
                      style: 'currency',
                      currency: 'USD',
                      maximumSignificantDigits: 3,
                    }}
                    onChange={(value) => setCostPerDeveloper(value[0])}
                  />

                  <NumberInput
                    label="Annual Work Weeks"
                    maxValue={52}
                    minValue={0}
                    value={workWeeks}
                    onChange={setWorkWeeks}
                  />
                  <NumberInput
                    label="Developer Cost per minute"
                    isDisabled={true}
                    formatOptions={{
                      style: 'currency',
                      currency: 'USD',
                      minimumFractionDigits: 2,
                    }}
                    value={costPerDeveloper / (workWeeks * minutesPerWorkWeek)}
                  />
                </Card>
                <Card>
                  <NumberInput
                    label="Approximate local builds per day per developer"
                    value={devBuildsPerDay}
                    onChange={setDevBuildsPerDay}
                  />

                  <NumberInput
                    label="Total local builds per week"
                    isDisabled={true}
                    value={devBuildsPerDay * 5 * devCount}
                  />
                  <NumberInput
                    label="Total CI builds per week"
                    value={ciBuildsPerWeek}
                    onChange={setCiBuildsPerWeek}
                  />
                </Card>
              </div>
            </div>
            <div className="space-y-12">
              <div className="max-w-4xl mx-auto sm:text-center">
                <H2>Speed up Slow Builds</H2>
                <p className="mt-3 text-xl text-gray-500 sm:mt-4 sm:text-center">
                  Turborepo gives you the tooling and services to speed up
                  builds, including a remote build cache that shares unchanged
                  artifacts across the entire team.
                </p>
              </div>
              <div className="space-y-12">
                <Card>
                  <div className="space-y-8">
                    <H3>Speed Up Local Development</H3>
                    <div className="grid grid-cols-1 lg:grid-cols-3 lg:grid-flow-row gap-5">
                      <NumberInput
                        value={devBuildsPerDay * 5 * devCount}
                        label="Local builds per week (calculated from above)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={avgLocalBuildMinutes}
                        onChange={setAvgLocalBuildMinutes}
                        label="Average local build minutes (without data or cache)"
                      />
                      <Slider
                        label="% of build wait time reduced"
                        value={[percentLocalWaitTimeReduced]}
                        maxValue={1}
                        minValue={0}
                        step={0.01}
                        formatOptions={{
                          style: 'percent',
                        }}
                        onChange={(value) =>
                          setPercentLocalWaitTimeReduced(value[0])
                        }
                      />
                      <NumberInput
                        value={
                          avgLocalBuildMinutes * percentLocalWaitTimeReduced
                        }
                        label="Average local build minutes (with data and cache)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={savedLocalDevMinutesPerBuild}
                        label="Saved Developer minutes per local build with data and cache"
                        isDisabled={true}
                      />
                      <div />
                      <NumberInput
                        value={savedLocalDevHoursPerWeek}
                        gradient={true}
                        label="Saved Developer hours per week (all developers)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={savedLocalDevHoursPerWeekPerDev}
                        gradient={true}
                        formatOptions={{
                          maximumSignificantDigits: 3,
                        }}
                        label="Saved Developer hours per week (each developer)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={annualLocalDevCostRecapture}
                        formatOptions={{
                          style: 'currency',
                          currency: 'USD',
                          maximumSignificantDigits: 7,
                        }}
                        gradient={true}
                        label="Annual Development Cost Recaptured (all developers)"
                        isDisabled={true}
                      />
                    </div>
                  </div>
                </Card>
                <Card>
                  <div className="space-y-8">
                    <H3>Speed Up Continuous Integration</H3>
                    <div className="grid grid-cols-1 lg:grid-cols-3 lg:grid-flow-row gap-5 ">
                      <NumberInput
                        value={ciBuildsPerWeek}
                        label="Number of CI builds per week (calculated from above)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={avgCiBuildMinutes}
                        label="Average CI build minutes (without data or cache)"
                        onChange={setAvgCiBuildMinutes}
                      />

                      <Slider
                        label="% of build wait time reduced"
                        value={[percentCiWaitTimeReduced]}
                        maxValue={1}
                        minValue={0}
                        step={0.01}
                        formatOptions={{
                          style: 'percent',
                        }}
                        onChange={(value) =>
                          setPercentCiWaitTimeReduced(value[0])
                        }
                      />
                      <Slider
                        label="% of CI builds where devs are waiting"
                        value={[percentCiDevWaitTimeReduced]}
                        maxValue={1}
                        minValue={0}
                        step={0.01}
                        formatOptions={{
                          style: 'percent',
                        }}
                        onChange={(value) =>
                          setPercentCiDevWaitTimeReduced(value[0])
                        }
                      />
                      <NumberInput
                        value={
                          avgCiBuildMinutes * (1 - percentCiWaitTimeReduced)
                        }
                        label="Average CI build minutes (with data and cache)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={savedCiMinutesPerBuild}
                        label="Saved Developer minutes with data and cache"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={savedCiHoursPerWeek}
                        gradient={true}
                        label="Saved Developer hours per week (all developers)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={savedCiHoursPerWeekPerDev}
                        gradient={true}
                        label="Saved Developer hours per week (each developer)"
                        isDisabled={true}
                      />
                      <NumberInput
                        value={
                          ((avgCiBuildMinutes *
                            (1 - percentCiWaitTimeReduced) *
                            ciBuildsPerWeek *
                            costPerDeveloper) /
                            minutesPerWorkWeek) *
                          percentCiDevWaitTimeReduced
                        }
                        formatOptions={{
                          style: 'currency',
                          currency: 'USD',
                          maximumSignificantDigits: 10,
                        }}
                        gradient={true}
                        label="Annual Development Cost Recaptured (all developers)"
                        isDisabled={true}
                      />
                    </div>
                  </div>
                </Card>
              </div>
            </div>
          </div>
          <div className="max-w-xl mx-auto text-center grid gap-5 grid-cols-1 sm:grid-cols-2 py-12">
            <button
              className="text-gray-500 py-2 px-3 inline-flex items-center justify-center rounded-md betterhover:hover:bg-white betterhover:hover:bg-opacity-5 duration-100 ease-in transition-all"
              onClick={shareLink}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 20 20"
                className="h-4 w-4 -ml-1 mr-2"
                fill="currentColor"
              >
                <path
                  fillRule="evenodd"
                  d="M12.586 4.586a2 2 0 112.828 2.828l-3 3a2 2 0 01-2.828 0 1 1 0 00-1.414 1.414 4 4 0 005.656 0l3-3a4 4 0 00-5.656-5.656l-1.5 1.5a1 1 0 101.414 1.414l1.5-1.5zm-5 5a2 2 0 012.828 0 1 1 0 101.414-1.414 4 4 0 00-5.656 0l-3 3a4 4 0 105.656 5.656l1.5-1.5a1 1 0 10-1.414-1.414l-1.5 1.5a2 2 0 11-2.828-2.828l3-3z"
                  clipRule="evenodd"
                />
              </svg>
              {didCopy
                ? `Copied to clipboard!`
                : `Share a link to these results`}
            </button>
            <a
              className="text-gray-500 py-2 px-3 inline-flex items-center justify-center rounded-md betterhover:hover:bg-white betterhover:hover:bg-opacity-5 duration-100 ease-in transition-all"
              href={`mailto:?${qs.stringify({
                subject: 'Turborepo ROI Calculation',
                body: `We could potentially save every developer on our team up to ${Intl.NumberFormat(
                  locale,
                  {
                    maximumSignificantDigits: 4,
                    minimumFractionDigits: 2,
                  }
                ).format(
                  savedLocalDevHoursPerWeekPerDev + savedCiHoursPerWeekPerDev
                )} hours per week by switching to Turborepo.\n\nCheck out this ROI calculation: https://turborepo.com${getUrl()}`,
              })}`}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 20 20"
                className="h-4 w-4 -ml-1 mr-2"
                fill="currentColor"
              >
                <path d="M2.003 5.884L10 9.882l7.997-3.998A2 2 0 0016 4H4a2 2 0 00-1.997 1.884z" />
                <path d="M18 8.118l-8 4-8-4V14a2 2 0 002 2h12a2 2 0 002-2V8.118z" />
              </svg>
              Email these results
            </a>
          </div>
        </Container>
      </>
    </Layout>
  )
}
function Results({
  locale,
  totalCostRecapture,
  annualSavedLocalDevHours,
  savedLocalDevHoursPerWeekPerDev,
  savedCiHoursPerWeekPerDev,
}: {
  locale: string
  totalCostRecapture: number
  annualSavedLocalDevHours: number
  savedLocalDevHoursPerWeekPerDev: number
  savedCiHoursPerWeekPerDev: number
}) {
  return (
    <dl className="max-w-lg lg:max-w-5xl mx-auto mt-8 grid grid-cols-1 gap-5 lg:grid-cols-3">
      <div className=" bg-gradient-to-r from-blue-500 to-red-500 overflow-hidden rounded-lg">
        <div className="px-4 py-5 lg:p-6 ">
          <dt className="text-sm font-medium text-white truncate ">
            Annual Development Cost Recapture
          </dt>
          <dd className="mt-1 text-3xl lg:text-4xl lg:text-5xl font-black text-white">
            {Intl.NumberFormat(locale, {
              style: 'currency',
              currency: 'USD',
              maximumSignificantDigits: 6,
            }).format(Math.round(totalCostRecapture))}
          </dd>
        </div>
      </div>
      <div className=" bg-gradient-to-r from-blue-500 to-red-500 overflow-hidden rounded-lg">
        <div className="px-4 py-5 lg:p-6 ">
          <dt className="text-sm font-medium text-white truncate">
            Annual Developer Hours Saved
          </dt>
          <dd className="mt-1 text-3xl lg:text-4xl lg:text-5xl font-black text-white">
            {Intl.NumberFormat(locale, {
              maximumSignificantDigits: 5,
              maximumFractionDigits: 2,
            }).format(annualSavedLocalDevHours)}{' '}
            <span className="text-xs">hours/year</span>
          </dd>
        </div>
      </div>
      <div className=" bg-gradient-to-r from-blue-500 to-red-500 overflow-hidden rounded-lg">
        <div className="px-4 py-5 lg:p-6 ">
          <dt className="text-sm font-medium text-white truncate">
            Each Developer Saves per Week
          </dt>
          <dd className="mt-1 text-3xl lg:text-4xl lg:text-5xl font-black text-white">
            {Intl.NumberFormat(locale, {
              maximumSignificantDigits: 4,
              minimumFractionDigits: 2,
            }).format(
              savedLocalDevHoursPerWeekPerDev + savedCiHoursPerWeekPerDev
            )}{' '}
            <span className="text-xs">hours/week</span>
          </dd>
        </div>
      </div>
    </dl>
  )
}
