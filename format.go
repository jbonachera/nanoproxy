package main

import (
	"time"
)

func humanDuration(d time.Duration) string {
	return d.Truncate(1 * time.Millisecond).String()
}
