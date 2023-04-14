// Package turbostate holds all of the state given from the Rust CLI
// that is necessary to execute turbo. We transfer this state from Rust
// to Go via a JSON payload.
package turbostate

import (
	"fmt"

	"github.com/vercel/turbo/cli/internal/util"
)

// RepoState is the state for repository. Consists of the root for the repo
// along with the mode (single package or multi package)
type RepoState struct {
	Root string `json:"root"`
	Mode string `json:"mode"`
}

// DaemonPayload is the extra flags and command that are
// passed for the `daemon` subcommand
type DaemonPayload struct {
	IdleTimeout string `json:"idle_time"`
	JSON        bool   `json:"json"`
}

// PrunePayload is the extra flags passed for the `prune` subcommand
type PrunePayload struct {
	Scope     []string `json:"scope"`
	Docker    bool     `json:"docker"`
	OutputDir string   `json:"output_dir"`
}

// RunPayload is the extra flags passed for the `run` subcommand
type RunPayload struct {
	CacheDir          string       `json:"cache_dir"`
	CacheWorkers      int          `json:"cache_workers"`
	Concurrency       string       `json:"concurrency"`
	ContinueExecution bool         `json:"continue_execution"`
	DryRun            string       `json:"dry_run"`
	Filter            []string     `json:"filter"`
	Force             bool         `json:"force"`
	GlobalDeps        []string     `json:"global_deps"`
	EnvMode           util.EnvMode `json:"env_mode"`
	// NOTE: Graph has three effective states that is modeled using a *string:
	//   nil -> no flag passed
	//   ""  -> flag passed but no file name attached: print to stdout
	//   "foo" -> flag passed and file name attached: emit to file
	// The mirror for this in Rust is `Option<String>` with the default value
	// for the flag being `Some("")`.
	Graph               *string  `json:"graph"`
	Ignore              []string `json:"ignore"`
	IncludeDependencies bool     `json:"include_dependencies"`
	NoCache             bool     `json:"no_cache"`
	NoDaemon            bool     `json:"no_daemon"`
	NoDeps              bool     `json:"no_deps"`
	Only                bool     `json:"only"`
	OutputLogs          string   `json:"output_logs"`
	PassThroughArgs     []string `json:"pass_through_args"`
	Parallel            bool     `json:"parallel"`
	Profile             string   `json:"profile"`
	RemoteOnly          bool     `json:"remote_only"`
	Scope               []string `json:"scope"`
	Since               string   `json:"since"`
	SinglePackage       bool     `json:"single_package"`
	Summarize           bool     `json:"summarize"`
	Tasks               []string `json:"tasks"`
	PkgInferenceRoot    string   `json:"pkg_inference_root"`
	LogPrefix           string   `json:"log_prefix"`
	ExperimentalSpaceID string   `json:"experimental_space_id"`
}

// Command consists of the data necessary to run a command.
// Only one of these fields should be initialized at a time.
type Command struct {
	Daemon *DaemonPayload `json:"daemon"`
	Prune  *PrunePayload  `json:"prune"`
	Run    *RunPayload    `json:"run"`
}

// ParsedArgsFromRust are the parsed command line arguments passed
// from the Rust shim
type ParsedArgsFromRust struct {
	API                string  `json:"api"`
	Color              bool    `json:"color"`
	CPUProfile         string  `json:"cpu_profile"`
	CWD                string  `json:"cwd"`
	Heap               string  `json:"heap"`
	Login              string  `json:"login"`
	NoColor            bool    `json:"no_color"`
	Preflight          bool    `json:"preflight"`
	RemoteCacheTimeout uint64  `json:"remote_cache_timeout"`
	Team               string  `json:"team"`
	Token              string  `json:"token"`
	Trace              string  `json:"trace"`
	Verbosity          int     `json:"verbosity"`
	TestRun            bool    `json:"test_run"`
	Command            Command `json:"command"`
}

// GetColor returns the value of the `color` flag.
func (a ParsedArgsFromRust) GetColor() bool {
	return a.Color
}

// GetNoColor returns the value of the `token` flag.
func (a ParsedArgsFromRust) GetNoColor() bool {
	return a.NoColor
}

// GetLogin returns the value of the `login` flag.
func (a ParsedArgsFromRust) GetLogin() (string, error) {
	return a.Login, nil
}

// GetAPI returns the value of the `api` flag.
func (a ParsedArgsFromRust) GetAPI() (string, error) {
	return a.API, nil
}

// GetTeam returns the value of the `team` flag.
func (a ParsedArgsFromRust) GetTeam() (string, error) {
	return a.Team, nil
}

// GetToken returns the value of the `token` flag.
func (a ParsedArgsFromRust) GetToken() (string, error) {
	return a.Token, nil
}

// GetCwd returns the value of the `cwd` flag.
func (a ParsedArgsFromRust) GetCwd() (string, error) {
	return a.CWD, nil
}

// GetRemoteCacheTimeout returns the value of the `remote-cache-timeout` flag.
func (a ParsedArgsFromRust) GetRemoteCacheTimeout() (uint64, error) {
	if a.RemoteCacheTimeout != 0 {
		return a.RemoteCacheTimeout, nil
	}
	return 0, fmt.Errorf("no remote cache timeout provided")
}
