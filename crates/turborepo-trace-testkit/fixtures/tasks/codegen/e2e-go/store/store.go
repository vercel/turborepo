// Package store defines the storage interface.
package store

//go:generate mockgen -source=store.go -destination=../mocks/mock_store.go -package=mocks

// Store stores strings.
type Store interface {
	Get(key string) (string, error)
}
