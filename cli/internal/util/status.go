package util

import "fmt"

// CachingStatus represents the api server's perspective
// on whether remote caching should be allowed
type CachingStatus int

const (
	// CachingStatusDisabled indicates that the server will not accept or serve artifacts
	CachingStatusDisabled CachingStatus = iota
	// CachingStatusEnabled indicates that the server will accept and serve artifacts
	CachingStatusEnabled
	// CachingStatusOverLimit indicates that a usage limit has been hit and the
	// server will temporarily not accept or serve artifacts
	CachingStatusOverLimit
	// CachingStatusPaused indicates that a customer's spending has been paused and the
	// server will temporarily not accept or serve artifacts
	CachingStatusPaused
)

// CachingStatusFromString parses a raw string to a caching status enum value
func CachingStatusFromString(raw string) (CachingStatus, error) {
	switch raw {
	case "disabled":
		return CachingStatusDisabled, nil
	case "enabled":
		return CachingStatusEnabled, nil
	case "over_limit":
		return CachingStatusOverLimit, nil
	case "paused":
		return CachingStatusPaused, nil
	default:
		return CachingStatusDisabled, fmt.Errorf("unknown caching status: %v", raw)
	}
}

// CacheDisabledError is an error used to indicate that remote caching
// is not available.
type CacheDisabledError struct {
	Status  CachingStatus
	Message string
}

func (cd *CacheDisabledError) Error() string {
	return cd.Message
}
