// Package store defines the storage interface.
package store

//go:generate mockgen -source=$GOFILE -destination=../mocks/mock_$GOFILE -package=mocks

// Store stores.
type Store interface {
	Get(key string) (string, error)
}
