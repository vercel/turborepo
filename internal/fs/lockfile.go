package fs

import (
	"crypto/md5"
	"encoding/hex"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path"
	"regexp"
	"strings"

	"gopkg.in/yaml.v2"
)

type LockfileEntry struct {
	// resolved version for the particular entry based on the provided semver revision
	Version   string `yaml:"version"`
	Resolved  string `yaml:"resolved"`
	Integrity string `yaml:"integrity"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	Dependencies map[string]string `yaml:"dependencies,omitempty"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	OptionalDependencies map[string]string `yaml:"optionalDependencies,omitempty"`
}

type YarnLockfile map[string]*LockfileEntry

func md5sum(filePath string) (string, error) {
	file, err := os.Open(filePath)
	if err != nil {
		return "", err
	}
	defer file.Close()

	hash := md5.New()
	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}

// ReadLockfile will read `yarn.lock` into memory (either from the cache or fresh)
func ReadLockfile(cacheDir string) (*YarnLockfile, error) {
	var lockfile YarnLockfile
	var prettyLockFile = YarnLockfile{}
	hash, err := HashFile("yarn.lock")

	contentsOfLock, err := ioutil.ReadFile(path.Join(cacheDir, fmt.Sprintf("%v-turbo-lock.yaml", hash)))
	if err != nil {
		contentsB, err := ioutil.ReadFile("yarn.lock")
		if err != nil {
			return nil, fmt.Errorf("yarn.lock: %w", err)
		}
		lines := strings.Split(string(contentsB), "\n")
		r := regexp.MustCompile(`^[\w"]`)
		double := regexp.MustCompile(`\:\"\:`)
		l := regexp.MustCompile("\"|:\n$")
		o := regexp.MustCompile(`\"\s\"`)
		// deals with colons
		// integrity sha-... -> integrity: sha-...
		// "@apollo/client" latest -> "@apollo/client": latest
		// "@apollo/client" "0.0.0" -> "@apollo/client": "0.0.0"
		// apollo-client "0.0.0" -> apollo-client: "0.0.0"
		a := regexp.MustCompile(`(\w|\")\s(\"|\w)`)

		for i, line := range lines {
			if r.MatchString(line) {
				first := fmt.Sprintf("\"%v\":", l.ReplaceAllString(line, ""))
				lines[i] = double.ReplaceAllString(first, "\":")
			}
		}
		output := o.ReplaceAllString(strings.Join(lines, "\n"), "\": \"")

		next := a.ReplaceAllStringFunc(output, func(m string) string {
			parts := a.FindStringSubmatch(m)
			return fmt.Sprintf("%v: %v", parts[1], parts[2])
		})

		err = yaml.Unmarshal([]byte(next), &lockfile)
		if err != nil {
			return &YarnLockfile{}, err
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
			fmt.Println(err.Error())
			return &YarnLockfile{}, err
		}
		if err = EnsureDir(path.Join(cacheDir)); err != nil {
			fmt.Println(err.Error())
			return &YarnLockfile{}, err
		}
		if err = EnsureDir(path.Join(cacheDir, fmt.Sprintf("%v-turbo-lock.yaml", hash))); err != nil {
			fmt.Println(err.Error())
			return &YarnLockfile{}, err
		}
		if err = ioutil.WriteFile(path.Join(cacheDir, fmt.Sprintf("%v-turbo-lock.yaml", hash)), []byte(better), 0644); err != nil {
			fmt.Println(err.Error())
			return &YarnLockfile{}, err
		}
	} else {
		if contentsOfLock != nil {
			err = yaml.Unmarshal(contentsOfLock, &prettyLockFile)
			if err != nil {
				return &YarnLockfile{}, err
			}
		}
	}

	return &prettyLockFile, nil
}
