package util

import (
	"fmt"
	"math"
	"runtime"
	"strconv"
	"strings"

	"github.com/spf13/pflag"
)

var (
	// alias so we can mock in tests
	runtimeNumCPU = runtime.NumCPU
	// positive values check for +Inf
	_positiveInfinity = 1
)

func ParseConcurrency(concurrencyRaw string) (int, error) {
	if strings.HasSuffix(concurrencyRaw, "%") {
		if percent, err := strconv.ParseFloat(concurrencyRaw[:len(concurrencyRaw)-1], 64); err != nil {
			return 0, fmt.Errorf("invalid value for --concurrency CLI flag. This should be a number --concurrency=4 or percentage of CPU cores --concurrency=50%% : %w", err)
		} else {
			if percent > 0 && !math.IsInf(percent, _positiveInfinity) {
				return int(math.Max(1, float64(runtimeNumCPU())*percent/100)), nil
			} else {
				return 0, fmt.Errorf("invalid percentage value for --concurrency CLI flag. This should be a percentage of CPU cores, between 1%% and 100%% : %w", err)
			}
		}
	} else if i, err := strconv.Atoi(concurrencyRaw); err != nil {
		return 0, fmt.Errorf("invalid value for --concurrency CLI flag. This should be a positive integer greater than or equal to 1: %w", err)
	} else {
		if i >= 1 {
			return i, nil
		} else {
			return 0, fmt.Errorf("invalid value %v for --concurrency CLI flag. This should be a positive integer greater than or equal to 1", i)
		}
	}
}

// ConcurrencyValue allows pflag to accept either a number or percentage
// of available CPUs as a value for concurrency
type ConcurrencyValue struct {
	Value *int
	raw   string
}

var _ pflag.Value = &ConcurrencyValue{}

// String implements pflag.Value.String for ConcurrencyValue
func (cv *ConcurrencyValue) String() string {
	return cv.raw
}

// Set implements pflag.Value.Set for ConcurrencyValue
func (cv *ConcurrencyValue) Set(value string) error {
	parsed, err := ParseConcurrency(value)
	if err != nil {
		return err
	}
	cv.raw = value
	*cv.Value = parsed
	return nil
}

// Type implements pflag.Value.Type for ConcurrencyValue
func (cv *ConcurrencyValue) Type() string {
	return "number|percentage"
}
