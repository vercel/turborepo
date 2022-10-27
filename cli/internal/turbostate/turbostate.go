package turbostate

import "time"

// RepoState is the state for repository. Consists of the root for the repo
// along with the mode (single package or multi package)
type RepoState struct {
	Root string `json:"root"`
	Mode string `json:"mode"`
}

// DaemonPayload is the extra flags passed for the `daemon` subcommand
type DaemonPayload struct {
	IdleTimeout time.Duration `json:"idle_timeout"`
}

// LinkPayload is the extra flags passed for the `link` subcommand
type LinkPayload struct {
	DontModifyGitIgnore bool `json:"dont_modify_gitIgnore"`
}

// LoginPayload is the extra flags passed for the `login` subcommand
type LoginPayload struct {
	SsoTeam string `json:"sso_team"`
}

// PrunePayload is the extra flags passed for the `prune` subcommand
type PrunePayload struct {
	Scope     string `json:"scope"`
	Docker    bool   `json:"docker"`
	OutputDir string `json:"output_dir"`
}

// Command consists of the data necessary to run a command.
// Only one of these fields should be initialized at a time.
type Command struct {
	Daemon *DaemonPayload `json:"daemon"`
	Link   *LinkPayload   `json:"link"`
	Login  *LoginPayload  `json:"login"`
	Logout *struct{}      `json:"logout"`
	Prune  *PrunePayload  `json:"prune"`
	Unlink *struct{}      `json:"unlink"`
}

// Args are the parsed command line arguments passed
// from the Rust shim
type Args struct {
	Api        *string  `json:"api"`
	Color      bool     `json:"color"`
	Cpuprofile *string  `json:"cpuprofile"`
	Cwd        *string  `json:"cwd"`
	Heap       *string  `json:"heap"`
	Login      *string  `json:"login"`
	NoColor    bool     `json:"noColor"`
	Preflight  bool     `json:"preflight"`
	Team       *string  `json:"team"`
	Token      *string  `json:"token"`
	Trace      *string  `json:"trace"`
	Verbosity  *uint8   `json:"verbosity"`
	Command    *Command `json:"command"`
}

// TurboState is the entire state of an execution passed from the Rust side
type TurboState struct {
	RepoState  RepoState `json:"repo_state"`
	ParsedArgs Args      `json:"parsed_args"`
	RawArgs    []string  `json:"raw_args"`
}
