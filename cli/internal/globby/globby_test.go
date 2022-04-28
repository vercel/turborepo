package globby

import (
	"path/filepath"
	"reflect"
	"sort"
	"testing"

	"github.com/spf13/afero"
)

// setup prepares the test file system contents and returns the file system.
func setup(files []string) afero.IOFS {
	fs := afero.NewIOFS(afero.NewMemMapFs())

	for _, file := range files {
		// We don't need the handle, we don't need the error.
		// We'll know if it errors because the tests will not pass.
		// nolint:errcheck
		fs.Create(file)
	}

	return fs
}

func TestGlobFilesFs(t *testing.T) {
	type args struct {
		basePath        string
		includePatterns []string
		excludePatterns []string
	}
	tests := []struct {
		name    string
		files   []string
		args    args
		want    []string
		wantErr bool
	}{
		{
			name:  "hello world",
			files: []string{"/test.txt"},
			args: args{
				basePath:        "/",
				includePatterns: []string{"*.txt"},
				excludePatterns: []string{},
			},
			want: []string{"/test.txt"},
		},
		{
			name: "finding workspace package.json files",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/kitchen-sink/package.json",
				"/repos/some-app/tests/mocks/kitchen-sink/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/*/package.json", "apps/*/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			want: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
		},
		{
			name: "excludes unexpected workspace package.json files",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			want: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
		},
		{
			name: "nested packages work",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
				"/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/**/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			want: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
		},
		{
			name: "includes do not override excludes",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
				"/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/**/package.json", "tests/mocks/*/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			want: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
		},
		{
			name: "output globbing grabs the desired content",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/src/index.js",
				"/repos/some-app/public/src/css/index.css",
				"/repos/some-app/.turbo/turbo-build.log",
				"/repos/some-app/.turbo/somebody-touched-this-file-into-existence.txt",
				"/repos/some-app/.next/log.txt",
				"/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
				"/repos/some-app/public/dist/css/index.css",
				"/repos/some-app/public/dist/images/rick_astley.jpg",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{".turbo/turbo-build.log", "dist/**", ".next/**", "public/dist/**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
				"/repos/some-app/.next/log.txt",
				"/repos/some-app/.turbo/turbo-build.log",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
				"/repos/some-app/public/dist/css/index.css",
				"/repos/some-app/public/dist/images/rick_astley.jpg",
			},
		},
		{
			name: "passing ** captures all children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "passing just a directory captures no children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist"},
				excludePatterns: []string{},
			},
			want: []string{},
		},
		{
			name: "redundant includes do not duplicate",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/*", "dist/**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "exclude everything, include everything",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**"},
			},
			want: []string{},
		},
		{
			name: "passing just a directory to exclude prevents capture of children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{"dist/js"},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
			},
		},
		{
			name: "passing ** to exclude prevents capture of children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{"dist/js/**"},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
			},
		},
		{
			name: "exclude everything with folder . applies at base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"./"},
			},
			want: []string{},
		},
		{
			name: "exclude everything with traversal applies at a non-base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"./dist"},
			},
			want: []string{},
		},
		{
			name: "exclude everything with folder traversal (..) applies at base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"dist/../"},
			},
			want: []string{},
		},
		{
			name: "how do globs even work bad glob microformat",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/**/**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "directory traversal stops at base path",
			files: []string{
				"/repos/spanish-inquisition/index.html",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"../spanish-inquisition/**", "dist/**"},
				excludePatterns: []string{},
			},
			want:    []string{},
			wantErr: true,
		},
		{
			name: "globs and traversal and globs do not cross base path",
			files: []string{
				"/repos/spanish-inquisition/index.html",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/../../spanish-inquisition/**"},
				excludePatterns: []string{},
			},
			want:    []string{},
			wantErr: true,
		},
		{
			name: "traversal works within base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/js/../**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "self-references (.) work",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/./././**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "depth of 1 includes does not capture folders",
			files: []string{
				"/repos/some-app/package.json",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"*"},
				excludePatterns: []string{},
			},
			want: []string{"/repos/some-app/package.json"},
		},
		{
			name: "depth of 1 excludes prevents capturing folders",
			files: []string{
				"/repos/some-app/package.json",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"dist/*"},
			},
			want: []string{"/repos/some-app/package.json"},
		},
		{
			name: "No-trailing slash basePath works",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{},
			},
			want: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
	}
	for _, tt := range tests {
		fs := setup(tt.files)

		t.Run(tt.name, func(t *testing.T) {
			got, err := globFilesFs(fs, tt.args.basePath, tt.args.includePatterns, tt.args.excludePatterns)

			if (err != nil) != tt.wantErr {
				t.Errorf("globFilesFs() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			gotToSlash := make([]string, len(got))
			for index, path := range got {
				gotToSlash[index] = filepath.ToSlash(path)
			}

			sort.Strings(gotToSlash)

			if !reflect.DeepEqual(gotToSlash, tt.want) {
				t.Errorf("globFilesFs() = %v, want %v", gotToSlash, tt.want)
			}
		})
	}
}
