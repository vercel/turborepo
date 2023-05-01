package util

import "strings"

// EnvMode specifies if we will be using strict env vars
type EnvMode string

const (
	// Infer - infer environment variable constraints from turbo.json
	Infer EnvMode = "Infer"
	// Loose - environment variables are unconstrained
	Loose EnvMode = "Loose"
	// Strict - environment variables are limited
	Strict EnvMode = "Strict"
)

// MarshalText implements TextMarshaler for the struct.
func (s EnvMode) MarshalText() (text []byte, err error) {
	return []byte(strings.ToLower(string(s))), nil
}

// RunOpts holds the options that control the execution of a turbo run
type RunOpts struct {
	// Force execution to be serially one-at-a-time
	Concurrency int
	// Whether to execute in parallel (defaults to false)
	Parallel bool

	EnvMode EnvMode
	// The filename to write a perf profile.
	Profile string
	// If true, continue task executions even if a task fails.
	ContinueOnError bool
	PassThroughArgs []string
	// Restrict execution to only the listed task names. Default false
	Only bool
	// Dry run flags
	DryRun     bool
	DryRunJSON bool
	// Graph flags
	GraphDot      bool
	GraphFile     string
	NoDaemon      bool
	SinglePackage bool

	// logPrefix controls whether we should print a prefix in task logs
	LogPrefix string

	// Whether turbo should create a run summary
	Summarize bool

	ExperimentalSpaceID string
}
