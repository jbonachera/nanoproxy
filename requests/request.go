package requests

import (
	"io"
	"log"
)

func ProcessConnect(clientConn io.ReadWriter, upstreamConn io.ReadWriter) {
	readCh := make(chan struct{})
	go func() {
		defer close(readCh)
		io.Copy(upstreamConn, clientConn)
	}()
	_, err := io.Copy(clientConn, upstreamConn)
	if err != nil {
		log.Printf("CONNECT: write failed: %v", err)
	}
	<-readCh
}
