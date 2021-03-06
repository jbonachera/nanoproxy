package main

import (
	"fmt"
	"time"
)

type kind int

const (
	connAdded kind = iota
	connRemoved
)

type event struct {
	kind kind
	conn *metricConn
}

type stats struct {
	events chan event
	conn   []*metricConn
}

func runStats() chan event {
	ch := make(chan event, 20)
	stats := &stats{}
	go func() {
		ticker := time.NewTicker(300 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ticker.C:
				/*for _, conn := range stats.conn {
					fmt.Printf("%s %s\n", conn.host, humanDuration(time.Since(conn.startedAt)))
				}*/
			case event := <-ch:
				switch event.kind {
				case connAdded:
					stats.conn = append(stats.conn, event.conn)
				case connRemoved:
					fmt.Printf("%s %s%s (%s %s)\n",
						event.conn.remote.method, event.conn.remote.host, event.conn.remote.path,
						humanDuration(time.Since(event.conn.startedAt)),
						humanBytes(event.conn.readBytes+event.conn.writtenBytes))
					for idx, conn := range stats.conn {
						if conn == event.conn {
							stats.conn = append(stats.conn[:idx], stats.conn[idx+1:]...)
							break
						}
					}
				}
			}
		}
	}()
	return ch
}
