package main

import (
	"fmt"
	"time"
)

func humanDuration(d time.Duration) string {
	return d.Truncate(1 * time.Millisecond).String()
}

var bytesUnit []string = []string{
	"o",
	"o",
	"go",
	"to",
}

func humanBytes(v uint64) string {
	for _, unit := range bytesUnit {
		if v <= 1000 {
			return fmt.Sprintf("%d%s", v, unit)
		}
		v = v / 1000
	}
	return ""
}
