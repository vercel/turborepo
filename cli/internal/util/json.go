package util

import (
	"encoding/json"
	"io/ioutil"

	"github.com/pkg/errors"
)

// ReadFileJSON reads json from the given path.
func ReadFileJSON(path string, v interface{}) error {
	b, err := ioutil.ReadFile(path)
	if err != nil {
		return errors.Wrap(err, "reading")
	}

	if err := json.Unmarshal(b, &v); err != nil {
		return errors.Wrap(err, "unmarshaling")
	}

	return nil
}
