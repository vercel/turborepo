package spinner

import (
	"context"
	"fmt"
	"io"
	"time"

	"github.com/mitchellh/cli"
	progressbar "github.com/schollz/progressbar/v3"
	"github.com/vercel/turbo/cli/internal/ui"
)

// getWriterAndColor unwraps cli.Ui instances until it gets to a BasicUi.
// If it happens to spot a ColoredUi along the way, it marks that color is
// enabled.
func getWriterAndColor(terminal cli.Ui, useColor bool) (io.Writer, bool) {
	switch terminal := terminal.(type) {
	case *cli.BasicUi:
		return terminal.Writer, useColor
	case *cli.ColoredUi:
		return getWriterAndColor(terminal.Ui, true)
	case *cli.ConcurrentUi:
		return getWriterAndColor(terminal.Ui, useColor)
	case *cli.PrefixedUi:
		return getWriterAndColor(terminal.Ui, useColor)
	case *cli.MockUi:
		return terminal.OutputWriter, false
	default:
		panic(fmt.Sprintf("unknown Ui: %v", terminal))
	}
}

// WaitFor runs fn, and prints msg to the terminal if it takes longer
// than initialDelay to complete. Depending on the terminal configuration, it may
// display a single instance of msg, or an infinite spinner, updated every 250ms.
func WaitFor(ctx context.Context, fn func(), terminal cli.Ui, msg string, initialDelay time.Duration) error {
	doneCh := make(chan struct{})
	go func() {
		fn()
		close(doneCh)
	}()
	if ui.IsTTY {
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(initialDelay):
			writer, useColor := getWriterAndColor(terminal, false)
			bar := progressbar.NewOptions(
				-1,
				progressbar.OptionEnableColorCodes(useColor),
				progressbar.OptionSetDescription(fmt.Sprintf("[yellow]%v[reset]", msg)),
				progressbar.OptionSpinnerType(14),
				progressbar.OptionSetWriter(writer),
			)
			for {
				select {
				case <-doneCh:
					err := bar.Finish()
					terminal.Output("")
					return err
				case <-time.After(250 * time.Millisecond):
					if err := bar.Add(1); err != nil {
						return err
					}
				case <-ctx.Done():
					return nil
				}
			}
		case <-doneCh:
			return nil
		}
	} else {
		// wait for the timeout before displaying a message, even with no tty
		select {
		case <-ctx.Done():
			return nil
		case <-doneCh:
			return nil
		case <-time.After(initialDelay):
			terminal.Output(msg)
		}
		select {
		case <-ctx.Done():
		case <-doneCh:
		}
		return nil
	}
}
