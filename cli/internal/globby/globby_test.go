package globby

import (
	"reflect"
	"testing"
)

func TestGlobFiles(t *testing.T) {

	type args struct {
		ws_path         string
		include_pattens []string
		exclude_pattens []string
	}
	tests := []struct {
		name string
		args args
		want []string
	}{
		{
			name: "globFiles",
			args: args{
				ws_path:         "testdata",
				include_pattens: []string{"package/**/*.go"},
				exclude_pattens: []string{"**/node_modules/**"},
			},
			want: []string{
				"package_deps_hash.go",
			},
		},
		// TODO: Add test cases.
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := GlobFiles(tt.args.ws_path, tt.args.include_pattens, tt.args.exclude_pattens); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GlobFiles() = %#v, want %#v", got, tt.want)
			}
		})
	}
}
