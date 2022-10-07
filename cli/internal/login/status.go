package login

import (
	"github.com/AlecAivazis/survey/v2"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/util"
)

var (
	errOverage            = errors.New("usage limit")
	errPaused             = errors.New("spending paused")
	errNeedCachingEnabled = errors.New("caching not enabled")
	errTryAfterEnable     = errors.New("link after enabling caching")
)

func promptEnableCaching() (bool, error) {
	shouldEnable := false
	err := survey.AskOne(
		&survey.Confirm{
			Default: true,
			Message: util.Sprintf("Remote Caching was previously disabled for this team. Would you like to enable it now?"),
		},
		&shouldEnable,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			icons.Question.Format = "gray+hb"
		}),
	)
	if err != nil {
		return false, err
	}
	return shouldEnable, nil
}
