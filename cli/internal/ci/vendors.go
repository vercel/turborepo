package ci

type vendorEnvs struct {
	Any []string
	All []string
}

// Vendor describes a CI/CD vendor execution environment
type Vendor struct {
	// Name is the name of the vendor
	Name string
	// Constant is the environment variable prefix used by the vendor
	Constant string
	// Env is one or many environment variables that can be used to quickly determine the vendor (using simple os.Getenv(env) check)
	Env vendorEnvs
	// EvalEnv is key/value map of environment variables that can be used to quickly determine the vendor
	EvalEnv map[string]string

	// The name of the environment variable that contains the current git sha
	ShaEnvVar string

	// The name of the environment variable that contains the current checked out branch
	BranchEnvVar string
}

// Vendors is a list of common CI/CD vendors (from https://github.com/watson/ci-info/blob/master/vendors.json)
var Vendors = []Vendor{
	{
		Name:     "Appcircle",
		Constant: "APPCIRCLE",
		Env:      vendorEnvs{Any: []string{"AC_APPCIRCLE"}},
	},
	{
		Name:     "AppVeyor",
		Constant: "APPVEYOR",
		Env:      vendorEnvs{Any: []string{"APPVEYOR"}},
	},
	{
		Name:     "AWS CodeBuild",
		Constant: "CODEBUILD",
		Env:      vendorEnvs{Any: []string{"CODEBUILD_BUILD_ARN"}},
	},
	{
		Name:     "Azure Pipelines",
		Constant: "AZURE_PIPELINES",
		Env:      vendorEnvs{Any: []string{"SYSTEM_TEAMFOUNDATIONCOLLECTIONURI"}},
	},
	{
		Name:     "Bamboo",
		Constant: "BAMBOO",
		Env:      vendorEnvs{Any: []string{"bamboo_planKey"}},
	},
	{
		Name:     "Bitbucket Pipelines",
		Constant: "BITBUCKET",
		Env:      vendorEnvs{Any: []string{"BITBUCKET_COMMIT"}},
	},
	{
		Name:     "Bitrise",
		Constant: "BITRISE",
		Env:      vendorEnvs{Any: []string{"BITRISE_IO"}},
	},
	{
		Name:     "Buddy",
		Constant: "BUDDY",
		Env:      vendorEnvs{Any: []string{"BUDDY_WORKSPACE_ID"}},
	},
	{
		Name:     "Buildkite",
		Constant: "BUILDKITE",
		Env:      vendorEnvs{Any: []string{"BUILDKITE"}},
	},
	{
		Name:     "CircleCI",
		Constant: "CIRCLE",
		Env:      vendorEnvs{Any: []string{"CIRCLECI"}},
	},
	{
		Name:     "Cirrus CI",
		Constant: "CIRRUS",
		Env:      vendorEnvs{Any: []string{"CIRRUS_CI"}},
	},
	{
		Name:     "Codefresh",
		Constant: "CODEFRESH",
		Env:      vendorEnvs{Any: []string{"CF_BUILD_ID"}},
	},
	{
		Name:     "Codemagic",
		Constant: "CODEMAGIC",
		Env:      vendorEnvs{Any: []string{"CM_BUILD_ID"}},
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
		Env:      vendorEnvs{Any: []string{"DRONE"}},
	},
	{
		Name:     "dsari",
		Constant: "DSARI",
		Env:      vendorEnvs{Any: []string{"DSARI"}},
	},
	{
		Name:     "Expo Application Services",
		Constant: "EAS",
		Env:      vendorEnvs{Any: []string{"EAS_BUILD"}},
	},
	{
		Name:         "GitHub Actions",
		Constant:     "GITHUB_ACTIONS",
		Env:          vendorEnvs{Any: []string{"GITHUB_ACTIONS"}},
		ShaEnvVar:    "GITHUB_SHA",
		BranchEnvVar: "GITHUB_REF_NAME",
	},
	{
		Name:     "GitLab CI",
		Constant: "GITLAB",
		Env:      vendorEnvs{Any: []string{"GITLAB_CI"}},
	},
	{
		Name:     "GoCD",
		Constant: "GOCD",
		Env:      vendorEnvs{Any: []string{"GO_PIPELINE_LABEL"}},
	},
	{
		Name:     "Google Cloud Build",
		Constant: "GOOGLE_CLOUD_BUILD",
		Env:      vendorEnvs{Any: []string{"BUILDER_OUTPUT"}},
	},
	{
		Name:     "LayerCI",
		Constant: "LAYERCI",
		Env:      vendorEnvs{Any: []string{"LAYERCI"}},
	},
	{
		Name:     "Gerrit",
		Constant: "GERRIT",
		Env:      vendorEnvs{Any: []string{"GERRIT_PROJECT"}},
	},
	{
		Name:     "Hudson",
		Constant: "HUDSON",
		Env:      vendorEnvs{Any: []string{"HUDSON"}},
	},
	{
		Name:     "Jenkins",
		Constant: "JENKINS",
		Env:      vendorEnvs{All: []string{"JENKINS_URL", "BUILD_ID"}},
	},
	{
		Name:     "Magnum CI",
		Constant: "MAGNUM",
		Env:      vendorEnvs{Any: []string{"MAGNUM"}},
	},
	{
		Name:     "Netlify CI",
		Constant: "NETLIFY",
		Env:      vendorEnvs{Any: []string{"NETLIFY"}},
	},
	{
		Name:     "Nevercode",
		Constant: "NEVERCODE",
		Env:      vendorEnvs{Any: []string{"NEVERCODE"}},
	},
	{
		Name:     "ReleaseHub",
		Constant: "RELEASEHUB",
		Env:      vendorEnvs{Any: []string{"RELEASE_BUILD_ID"}},
	},
	{
		Name:     "Render",
		Constant: "RENDER",
		Env:      vendorEnvs{Any: []string{"RENDER"}},
	},
	{
		Name:     "Sail CI",
		Constant: "SAIL",
		Env:      vendorEnvs{Any: []string{"SAILCI"}},
	},
	{
		Name:     "Screwdriver",
		Constant: "SCREWDRIVER",
		Env:      vendorEnvs{Any: []string{"SCREWDRIVER"}},
	},
	{
		Name:     "Semaphore",
		Constant: "SEMAPHORE",
		Env:      vendorEnvs{Any: []string{"SEMAPHORE"}},
	},
	{
		Name:     "Shippable",
		Constant: "SHIPPABLE",
		Env:      vendorEnvs{Any: []string{"SHIPPABLE"}},
	},
	{
		Name:     "Solano CI",
		Constant: "SOLANO",
		Env:      vendorEnvs{Any: []string{"TDDIUM"}},
	},
	{
		Name:     "Sourcehut",
		Constant: "SOURCEHUT",
		EvalEnv: map[string]string{
			"CI_NAME": "sourcehut",
		},
	},
	{
		Name:     "Strider CD",
		Constant: "STRIDER",
		Env:      vendorEnvs{Any: []string{"STRIDER"}},
	},
	{
		Name:     "TaskCluster",
		Constant: "TASKCLUSTER",
		Env:      vendorEnvs{All: []string{"TASK_ID", "RUN_ID"}},
	},
	{
		Name:     "TeamCity",
		Constant: "TEAMCITY",
		Env:      vendorEnvs{Any: []string{"TEAMCITY_VERSION"}},
	},
	{
		Name:     "Travis CI",
		Constant: "TRAVIS",
		Env:      vendorEnvs{Any: []string{"TRAVIS"}},
	},
	{
		Name:         "Vercel",
		Constant:     "VERCEL",
		Env:          vendorEnvs{Any: []string{"NOW_BUILDER", "VERCEL"}},
		ShaEnvVar:    "VERCEL_GIT_COMMIT_SHA",
		BranchEnvVar: "VERCEL_GIT_COMMIT_REF",
	},
	{
		Name:     "Visual Studio App Center",
		Constant: "APPCENTER",
		Env:      vendorEnvs{Any: []string{"APPCENTER"}},
	},
	{
		Name:     "Woodpecker",
		Constant: "WOODPECKER",
		EvalEnv: map[string]string{
			"CI": "woodpecker",
		},
	},
	{
		Name:     "Xcode Cloud",
		Constant: "XCODE_CLOUD",
		Env:      vendorEnvs{Any: []string{"CI_XCODE_PROJECT"}},
	},
	{
		Name:     "Xcode Server",
		Constant: "XCODE_SERVER",
		Env:      vendorEnvs{Any: []string{"XCS"}},
	},
}
