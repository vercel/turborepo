package util

import (
	"fmt"
	"math"
	"runtime"
	"strconv"
	"strings"
)

var (
	// alias so we can mock in tests
	runtimeNumCPU = runtime.NumCPU
)

func ParseConcurrency(concurrencyRaw string) (int, error) {
	if strings.HasSuffix(concurrencyRaw, "%") {
		if percent, err := strconv.ParseFloat(concurrencyRaw[:len(concurrencyRaw)-1], 64); err != nil {
			return 0, fmt.Errorf("invalid value for --concurrency CLI flag. This should be a number --concurrency=4 or percentage of CPU cores --concurrency=50%% : %w", err)
		} else {
			if percent > 0 {
				return int(math.Max(1, float64(runtimeNumCPU())*percent/100)), nil
			} else {
				return 0, fmt.Errorf("invalid percentage value for --concurrency CLI flag. This should be a percentage of CPU cores, betteen 1%% and 100%% : %w", err)
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
