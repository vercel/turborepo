package fs

import (
	"bytes"
	"fmt"
	"io/ioutil"
	"path/filepath"
	"regexp"
	"strings"

	"gopkg.in/yaml.v3"
)

var rnLineEnding = regexp.MustCompile("\"|:\r\n$")
var nLineEnding = regexp.MustCompile("\"|:\n$")
var r = regexp.MustCompile(`^[\w"]`)
var double = regexp.MustCompile(`\:\"\:`)
var o = regexp.MustCompile(`\"\s\"`)

// deals with colons
// integrity sha-... -> integrity: sha-...
// "@apollo/client" latest -> "@apollo/client": latest
// "@apollo/client" "0.0.0" -> "@apollo/client": "0.0.0"
// apollo-client "0.0.0" -> apollo-client: "0.0.0"
var a = regexp.MustCompile(`(\w|\")\s(\"|\w)`)

// ReadLockfile will read `yarn.lock` into memory (either from the cache or fresh)
func ReadLockfile(rootpath string, backendName string, cacheDir AbsolutePath) (*YarnLockfile, error) {
	var lockfile YarnLockfile
	var prettyLockFile = YarnLockfile{}
	hash, err := HashFile(filepath.Join(rootpath, "yarn.lock"))
	if err != nil {
		return &YarnLockfile{}, fmt.Errorf("failed to hash lockfile: %w", err)
	}
	turboLockFile := cacheDir.Join(fmt.Sprintf("%v-turbo-lock.yaml", hash))
	contentsOfLock, err := turboLockFile.ReadFile()
	if err != nil {
		contentsB, err := ioutil.ReadFile(filepath.Join(rootpath, "yarn.lock"))
		if err != nil {
			return nil, fmt.Errorf("reading yarn.lock: %w", err)
		}

		var next []byte
		if backendName == "nodejs-yarn" {
			var lines []string
			var l *regexp.Regexp
			var output string

			hasLF := !bytes.HasSuffix(contentsB, []byte("\r\n"))
			if hasLF {
				lines = strings.Split(string(contentsB), "\n")
				l = nLineEnding
			} else {
				lines = strings.Split(strings.TrimRight(string(contentsB), "\r\n"), "\r\n")
				l = rnLineEnding
			}

			for i, line := range lines {
				if r.MatchString(line) {
					first := fmt.Sprintf("\"%v\":", l.ReplaceAllString(line, ""))
					lines[i] = double.ReplaceAllString(first, "\":")
				}
			}

			if hasLF {
				output = o.ReplaceAllString(strings.Join(lines, "\n"), "\": \"")
			} else {
				output = o.ReplaceAllString(strings.Join(lines, "\r\n"), "\": \"")
			}

			next = []byte(a.ReplaceAllStringFunc(output, func(m string) string {
				parts := a.FindStringSubmatch(m)
				return fmt.Sprintf("%s: %s", parts[1], parts[2])
			}))
		} else {
			next = contentsB
		}

		err = yaml.Unmarshal(next, &lockfile)
		if err != nil {
			return &YarnLockfile{}, fmt.Errorf("could not unmarshal lockfile: %w", err)
		}
		// This final step is important, it splits any deps with multiple-resolutions
		// (e.g. "@babel/generator@^7.13.0, @babel/generator@^7.13.9":) into separate
		// entries in our map
		// TODO: make concurrent
		for key, val := range lockfile {
			if strings.Contains(key, ",") {
				for _, v := range strings.Split(key, ", ") {
					prettyLockFile[strings.TrimSpace(v)] = val
				}

			} else {
				prettyLockFile[key] = val
			}
		}

		better, err := yaml.Marshal(&prettyLockFile)
		if err != nil {
			return nil, err
		}
		if err = turboLockFile.EnsureDir(); err != nil {
			return nil, err
		}
		if err = turboLockFile.WriteFile([]byte(better), 0644); err != nil {
			return nil, err
		}
	} else {
		if contentsOfLock != nil {
			err = yaml.Unmarshal(contentsOfLock, &prettyLockFile)
			if err != nil {
				return &YarnLockfile{}, fmt.Errorf("could not unmarshal yaml: %w", err)
			}
		}
	}

	return &prettyLockFile, nil
}
