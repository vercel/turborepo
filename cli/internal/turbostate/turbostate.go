// Package turbostate holds all of the state given from the Rust CLI
// that is necessary to execute turbo. We transfer this state from Rust
// to Go via a JSON payload.
package turbostate

// RepoState is the state for repository. Consists of the root for the repo
// along with the mode (single package or multi package)
type RepoState struct {
	Root string `json:"root"`
	Mode string `json:"mode"`
}

// LinkPayload is the extra flags passed for the `link` subcommand
type LinkPayload struct {
	DontModifyGitIgnore bool `json:"no_gitignore"`
}

// LoginPayload is the extra flags passed for the `login` subcommand
type LoginPayload struct {
	SsoTeam string `json:"sso_team"`
}

// Command consists of the data necessary to run a command.
// Only one of these fields should be initialized at a time.
type Command struct {
	Link   *LinkPayload  `json:"link"`
	Login  *LoginPayload `json:"login"`
	Logout *struct{}     `json:"logout"`
	Unlink *struct{}     `json:"unlink"`
}

// ParsedArgsFromRust are the parsed command line arguments passed
// from the Rust shim
type ParsedArgsFromRust struct {
	API        string   `json:"api"`
	Color      bool     `json:"color"`
	CPUProfile string   `json:"cpu_profile"`
	CWD        string   `json:"cwd"`
	Heap       string   `json:"heap"`
	Login      string   `json:"login"`
	NoColor    bool     `json:"no_color"`
	Preflight  bool     `json:"preflight"`
	Team       string   `json:"team"`
	Token      string   `json:"token"`
	Trace      string   `json:"trace"`
	Verbosity  uint8    `json:"verbosity"`
	TestRun    bool     `json:"test_run"`
	Command    *Command `json:"command"`
}

var _ config.CLIConfigProvider = (*ParsedArgsFromRust)(nil)

// CLIExecutionStateFromRust is the entire state of an execution passed from the Rust side
type CLIExecutionStateFromRust struct {
	RepoState  RepoState          `json:"repo_state"`
	ParsedArgs ParsedArgsFromRust `json:"parsed_args"`
	RawArgs    []string           `json:"raw_args"`
}

// GetColor returns the value of the `color` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetColor() bool {
	return a.Color
}

// GetNoColor returns the value of the `token` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetNoColor() bool {
	return a.NoColor
}

// GetLogin returns the value of the `login` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetLogin() (string, error) {
	return a.Login, nil
}

// GetAPI returns the value of the `api` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetAPI() (string, error) {
	return a.API, nil
}

// GetTeam returns the value of the `team` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetTeam() (string, error) {
	return a.Team, nil
}

// GetToken returns the value of the `token` flag. Used to implement CLIConfigProvider interface.
func (a ParsedArgsFromRust) GetToken() (string, error) {
	return a.Token, nil
}
