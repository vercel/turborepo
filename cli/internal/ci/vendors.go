package ci

// Vendor describes a CI/CD vendor execution environment
type Vendor struct {
	// Name is the name of the vendor
	Name string
	// Constant is the environment variable prefix used by the vendor
	Constant string
	// Env is an environment variable that can be used to quickly determine the vendor (using simple os.Getenv(env) check)
	Env string
	// EvalEnv is key/value map of environment variables that can be used to quickly determine the vendor
	EvalEnv map[string]string
}

// Vendors is a list of common CI/CD vendors
var Vendors = []Vendor{
	{
		Name:     "AppVeyor",
		Constant: "APPVEYOR",
		Env:      "APPVEYOR",
	},
	{
		Name:     "Azure Pipelines",
		Constant: "AZURE_PIPELINES",
		Env:      "SYSTEM_TEAMFOUNDATIONCOLLECTIONURI",
	},
	{
		Name:     "Appcircle",
		Constant: "APPCIRCLE",
		Env:      "AC_APPCIRCLE",
	},
	{
		Name:     "Bamboo",
		Constant: "BAMBOO",
		Env:      "bamboo_planKey",
	},
	{
		Name:     "Bitbucket Pipelines",
		Constant: "BITBUCKET",
		Env:      "BITBUCKET_COMMIT",
	},
	{
		Name:     "Bitrise",
		Constant: "BITRISE",
		Env:      "BITRISE_IO",
	},
	{
		Name:     "Buddy",
		Constant: "BUDDY",
		Env:      "BUDDY_WORKSPACE_ID",
	},
	{
		Name:     "Buildkite",
		Constant: "BUILDKITE",
		Env:      "BUILDKITE",
	},
	{
		Name:     "CircleCI",
		Constant: "CIRCLE",
		Env:      "CIRCLECI",
	},
	{
		Name:     "Cirrus CI",
		Constant: "CIRRUS",
		Env:      "CIRRUS_CI",
	},
	{
		Name:     "AWS CodeBuild",
		Constant: "CODEBUILD",
		Env:      "CODEBUILD_BUILD_ARN",
	},
	{
		Name:     "Codefresh",
		Constant: "CODEFRESH",
		Env:      "CF_BUILD_ID",
	},
	{
		Name:     "Codeship",
		Constant: "CODESHIP",
		EvalEnv: map[string]string{
			"CI_NAME": "codeship",
		},
	},
	{
		Name:     "Drone",
		Constant: "DRONE",
		Env:      "DRONE",
	},
	{
		Name:     "dsari",
		Constant: "DSARI",
		Env:      "DSARI",
	},
	{
		Name:     "GitHub Actions",
		Constant: "GITHUB_ACTIONS",
		Env:      "GITHUB_ACTIONS",
	},
	{
		Name:     "GitLab CI",
		Constant: "GITLAB",
		Env:      "GITLAB_CI",
	},
	{
		Name:     "GoCD",
		Constant: "GOCD",
		Env:      "GO_PIPELINE_LABEL",
	},
	{
		Name:     "LayerCI",
		Constant: "LAYERCI",
		Env:      "LAYERCI",
	},
	{
		Name:     "Hudson",
		Constant: "HUDSON",
		Env:      "HUDSON_URL",
	},
	{
		Name:     "Jenkins",
		Constant: "JENKINS",
		Env:      "JENKINS_URL",
	},
	{
		Name:     "Jenkins",
		Constant: "JENKINS",
		Env:      "BUILD_ID",
	},
	{
		Name:     "Magnum CI",
		Constant: "MAGNUM",
		Env:      "MAGNUM",
	},
	{
		Name:     "Netlify CI",
		Constant: "NETLIFY",
		Env:      "NETLIFY",
	},
	{
		Name:     "Nevercode",
		Constant: "NEVERCODE",
		Env:      "NEVERCODE",
	},
	{
		Name:     "Render",
		Constant: "RENDER",
		Env:      "RENDER",
	},
	{
		Name:     "Sail CI",
		Constant: "SAIL",
		Env:      "SAILCI",
	},
	{
		Name:     "Semaphore",
		Constant: "SEMAPHORE",
		Env:      "SEMAPHORE",
	},
	{
		Name:     "Screwdriver",
		Constant: "SCREWDRIVER",
		Env:      "SCREWDRIVER",
	},
	{
		Name:     "Shippable",
		Constant: "SHIPPABLE",
		Env:      "SHIPPABLE",
	},
	{
		Name:     "Solano CI",
		Constant: "SOLANO",
		Env:      "TDDIUM",
	},
	{
		Name:     "Strider CD",
		Constant: "STRIDER",
		Env:      "STRIDER",
	},
	{
		Name:     "TaskCluster",
		Constant: "TASKCLUSTER",
		Env:      "TASK_ID",
	},
	{
		Name:     "TaskCluster",
		Constant: "TASKCLUSTER",
		Env:      "RUN_ID",
	},
	{
		Name:     "TeamCity",
		Constant: "TEAMCITY",
		Env:      "TEAMCITY_VERSION",
	},
	{
		Name:     "Travis CI",
		Constant: "TRAVIS",
		Env:      "TRAVIS",
	},
	{
		Name:     "Vercel",
		Constant: "VERCEL",
		Env:      "NOW_BUILDER",
	},
	{
		Name:     "Visual Studio App Center",
		Constant: "APPCENTER",
		Env:      "APPCENTER_BUILD_ID",
	},
}
