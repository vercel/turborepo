package filter

import (
	"path/filepath"
	"reflect"
	"testing"
)

func TestParseTargetSelector(t *testing.T) {
	type args struct {
		rawSelector string
		prefix      string
	}
	tests := []struct {
		name    string
		args    args
		want    TargetSelector
		wantErr bool
	}{
		{
			"foo",
			args{"foo", "."},
			TargetSelector{
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
			args{"foo...", "."},
			TargetSelector{
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
			args{"...foo", "."},
			TargetSelector{
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
			args{"...foo...", "."},
			TargetSelector{
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
			args{"foo^...", "."},
			TargetSelector{
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
			args{"...^foo", "."},
			TargetSelector{
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
			args{"./foo", "./"},
			TargetSelector{
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
			args{"../foo", "."},
			TargetSelector{
				fromRef:             "",
				exclude:             false,
				excludeSelf:         false,
				includeDependencies: false,
				includeDependents:   false,
				namePattern:         "",
				parentDir:           filepath.FromSlash("../foo"),
			},
			false,
		},
		{
			"...{./foo}",
			args{"...{./foo}", "."},
			TargetSelector{
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
			args{".", "."},
			TargetSelector{
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
			args{"..", "."},
			TargetSelector{
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
			args{"[master]", "."},
			TargetSelector{
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
			args{"[from...to]", "."},
			TargetSelector{
				fromRef:       "from",
				toRefOverride: "to",
			},
			false,
		},
		{
			"{foo}[master]",
			args{"{foo}[master]", "."},
			TargetSelector{
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
			args{"pattern{foo}[master]", "."},
			TargetSelector{
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
			args{"[master]...", "."},
			TargetSelector{
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
			args{"...[master]", "."},
			TargetSelector{
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
			args{"...[master]...", "."},
			TargetSelector{
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
			args{"...[from...to]...", "."},
			TargetSelector{
				fromRef:             "from",
				toRefOverride:       "to",
				includeDependencies: true,
				includeDependents:   true,
			},
			false,
		},
		{
			"foo...[master]",
			args{"foo...[master]", "."},
			TargetSelector{
				fromRef:           "master",
				namePattern:       "foo",
				matchDependencies: true,
			},
			false,
		},
		{
			"foo...[master]...",
			args{"foo...[master]...", "."},
			TargetSelector{
				fromRef:             "master",
				namePattern:         "foo",
				matchDependencies:   true,
				includeDependencies: true,
			},
			false,
		},
		{
			"{foo}...[master]",
			args{"{foo}...[master]", "."},
			TargetSelector{
				fromRef:           "master",
				parentDir:         "foo",
				matchDependencies: true,
			},
			false,
		},
		{
			"......[master]",
			args{"......[master]", "."},
			TargetSelector{},
			true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := ParseTargetSelector(tt.args.rawSelector, tt.args.prefix)
			if tt.wantErr {
				if err == nil {
					t.Errorf("ParseTargetSelector() error = %#v, wantErr %#v", err, tt.wantErr)
				}
			} else {
				// copy the raw selector from the args into what we want. This value is used
				// for reporting errors in the case of a malformed selector
				tt.want.raw = tt.args.rawSelector
				if !reflect.DeepEqual(got, tt.want) {
					t.Errorf("ParseTargetSelector() = %#v, want %#v", got, tt.want)
				}
			}
		})
	}
}
