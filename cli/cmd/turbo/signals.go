package main

import (
	"os"
	"os/signal"
	"syscall"
)

func watchSignals(onClose func()) <-chan struct{} {
	// TODO: platform specific signals to watch for?
	doneCh := make(chan struct{})
	signalCh := make(chan os.Signal, 1)
	signal.Notify(signalCh, os.Interrupt, syscall.SIGTERM, syscall.SIGQUIT)
	go func() {
		<-signalCh
		onClose()
		close(doneCh)
	}()
	return doneCh
}
