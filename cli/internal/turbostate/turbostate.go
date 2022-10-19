package turbostate

import "time"

type RepoState struct {
	Root string `json:"root"`
	Mode string `json:"mode"`
}

type DaemonPayload struct {
	IdleTimeout time.Duration `json:"idle_timeout"`
}

type LinkPayload struct {
	DontModifyGitIgnore bool `json:"dont_modify_gitIgnore"`
}

type LoginPayload struct {
	SsoTeam string `json:"sso_team"`
}

type PrunePayload struct {
	Scope     string `json:"scope"`
	Docker    bool   `json:"docker"`
	OutputDir string `json:"output_dir"`
}

type Command struct {
	Daemon *DaemonPayload `json:"daemon"`
	Link   *LinkPayload   `json:"link"`
	Login  *LoginPayload  `json:"login"`
	Prune  *PrunePayload  `json:"prune"`
}

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

type TurboState struct {
	RepoState  RepoState `json:"repo_state"`
	ParsedArgs Args      `json:"parsed_args"`
	RawArgs    []string  `json:"raw_args"`
}
