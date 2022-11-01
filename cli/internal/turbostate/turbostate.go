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
	DontModifyGitIgnore bool `json:"dont_modify_gitIgnore"`
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

// Args are the parsed command line arguments passed
// from the Rust shim
type Args struct {
	API        string   `json:"api"`
	Color      bool     `json:"color"`
	CPUProfile *string  `json:"cpuprofile"`
	CWD        *string  `json:"cwd"`
	Heap       *string  `json:"heap"`
	Login      string   `json:"login"`
	NoColor    bool     `json:"noColor"`
	Preflight  bool     `json:"preflight"`
	Team       string   `json:"team"`
	Token      string   `json:"token"`
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

// GetToken returns the value of the `color` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetColor() bool {
	return a.Color
}

// GetToken returns the value of the `token` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetNoColor() bool {
	return a.NoColor
}

// GetLogin returns the value of the `login` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetLogin() (string, error) {
	return a.Login, nil
}

// GetAPI returns the value of the `api` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetAPI() (string, error) {
	return a.API, nil
}

// GetTeam returns the value of the `team` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetTeam() (string, error) {
	return a.Team, nil
}

// GetToken returns the value of the `token` flag. Used to implement CLIConfigProvider interface.
func (a Args) GetToken() (string, error) {
	return a.Token, nil
}
