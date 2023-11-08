package nodes

import (
	"testing"

	"gotest.tools/v3/assert"
)

func TestLogFilename(t *testing.T) {
	testCases := []struct{ input, want string }{
		{
			"build",
			"turbo-build.log",
		},
		{
			"build:prod",
			"turbo-build$colon$prod.log",
		},
		{
			"build:prod:extra",
			"turbo-build$colon$prod$colon$extra.log",
		},
	}

	for _, testCase := range testCases {
		got := logFilename(testCase.input)
		assert.Equal(t, got, testCase.want)
	}
}
