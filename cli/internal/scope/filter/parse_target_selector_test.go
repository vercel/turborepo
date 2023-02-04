package filter

import (
	"reflect"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

func TestParseTargetSelector(t *testing.T) {
	tests := []struct {
		rawSelector string
		want        *TargetSelector
		wantErr     bool
	}{
		{
			"{}",
			&TargetSelector{},
			true,
		},
		{
			"foo",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"foo...",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: true,
				includeDependents:   false,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"...foo",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   true,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"...foo...",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: true,
				includeDependents:   true,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"foo^...",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         true,
				includeDependencies: true,
				includeDependents:   false,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"...^foo",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         true,
				includeDependencies: false,
				includeDependents:   true,
				namePattern:         "foo",
				parentDir:           "",
			},
			false,
		},
		{
			"./foo",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           "foo",
			},
			false,
		},
		{
			"../foo",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           turbopath.MakeRelativeSystemPath("..", "foo"),
			},
			false,
		},
		{
			"...{./foo}",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   true,
				namePattern:         "",
				parentDir:           "foo",
			},
			false,
		},
		{
			".",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           ".",
			},
			false,
		},
		{
			"..",
			&TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           "..",
			},
			false,
		},
		{
			"[master]",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           "",
			},
			false,
		},
		{
			"[from...to]",
			&TargetSelector{
				fromRef:       "from",
				toRefOverride: "to",
			},
			false,
		},
		{
			"{foo}[master]",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           "foo",
			},
			false,
		},
		{
			"pattern{foo}[master]",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "pattern",
				parentDir:           "foo",
			},
			false,
		},
		{
			"[master]...",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: true,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           "",
			},
			false,
		},
		{
			"...[master]",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   true,
				namePattern:         "",
				parentDir:           "",
			},
			false,
		},
		{
			"...[master]...",
			&TargetSelector{
				fromRef:             "master",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: true,
				includeDependents:   true,
				namePattern:         "",
				parentDir:           "",
			},
			false,
		},
		{
			"...[from...to]...",
			&TargetSelector{
				fromRef:             "from",
				toRefOverride:       "to",
				includeDependencies: true,
				includeDependents:   true,
			},
			false,
		},
		{
			"foo...[master]",
			&TargetSelector{
				fromRef:           "master",
				namePattern:       "foo",
				matchDependencies: true,
			},
			false,
		},
		{
			"foo...[master]...",
			&TargetSelector{
				fromRef:             "master",
				namePattern:         "foo",
				matchDependencies:   true,
				includeDependencies: true,
			},
			false,
		},
		{
			"{foo}...[master]",
			&TargetSelector{
				fromRef:           "master",
				parentDir:         "foo",
				matchDependencies: true,
			},
			false,
		},
		{
			"......[master]",
			&TargetSelector{},
			true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.rawSelector, func(t *testing.T) {
			got, err := ParseTargetSelector(tt.rawSelector)
			if tt.wantErr {
				if err == nil {
					t.Errorf("ParseTargetSelector() error = %#v, wantErr %#v", err, tt.wantErr)
				}
			} else {
				// copy the raw selector from the args into what we want. This value is used
				// for reporting errors in the case of a malformed selector
				tt.want.raw = tt.rawSelector
				if !reflect.DeepEqual(got, tt.want) {
					t.Errorf("ParseTargetSelector() = %#v, want %#v", got, tt.want)
				}
			}
		})
	}
}
