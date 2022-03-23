package logger

import (
	"io"
	"os"
	"sync"
)

type ConcurrentLogger struct {
	logger *Logger
	Out    io.Writer
	mutex  sync.Mutex
}

func NewConcurrent(logger *Logger) *ConcurrentLogger {
	return &ConcurrentLogger{
		Out: os.Stdout,
	}
}

func (l *ConcurrentLogger) Printf(format string, args ...interface{}) {
	l.mutex.Lock()
	defer l.mutex.Unlock()

	l.logger.Printf(format, args...)
}

func (l *ConcurrentLogger) Sucessf(format string, args ...interface{}) string {
	l.mutex.Lock()
	defer l.mutex.Unlock()

	return l.logger.Sucessf(format, args...)
}

func (l *ConcurrentLogger) Warnf(format string, args ...interface{}) error {
	l.mutex.Lock()
	defer l.mutex.Unlock()

	return l.logger.Warnf(format, args...)
}

func (l *ConcurrentLogger) Errorf(format string, args ...interface{}) error {
	l.mutex.Lock()
	defer l.mutex.Unlock()

	return l.logger.Errorf(format, args...)
}
