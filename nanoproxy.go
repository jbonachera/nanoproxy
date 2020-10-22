package main

import (
	"bufio"
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"log"
	"net"
	"net/textproto"
	"net/url"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/spf13/viper"
)

func bidirectionalPipe(ctx context.Context, clientConn io.ReadWriter, upstreamConn io.ReadWriter) {
	readCh := make(chan struct{})
	writeCh := make(chan struct{})
	go func() {
		defer close(readCh)
		io.Copy(upstreamConn, clientConn)
	}()
	go func() {
		defer close(writeCh)
		io.Copy(clientConn, upstreamConn)
	}()
	select {
	case <-readCh:
	case <-writeCh:
	case <-ctx.Done():
	}
}

type connHandler func(ctx context.Context, conn io.ReadWriter)

func upstreamConnHandler(dialer net.Dialer, upstreamURL string) connHandler {
	upstream, err := url.Parse(upstreamURL)
	if err != nil {
		panic(err)
	}
	authString := []byte{}
	if user := upstream.User.String(); user != "" {
		auth := fmt.Sprintf("Basic %s", base64.StdEncoding.EncodeToString([]byte(upstream.User.String())))
		authString = []byte(fmt.Sprintf("Proxy-Authorization: %s\n", auth))
	}
	return func(ctx context.Context, conn io.ReadWriter) {
		upstreamConn, err := dialer.DialContext(ctx, "tcp", upstream.Host)
		if err != nil {
			log.Printf("WARN: %v", err)
			return
		}
		defer upstreamConn.Close()
		reader := bufio.NewReader(conn)
		txtproto := textproto.NewReader(reader)
		first := true
		for {
			buf, err := txtproto.ReadLineBytes()
			if first {
				log.Print(string(buf))
				first = false
			}
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			if len(buf) == 0 {
				break
			}
			_, err = upstreamConn.Write(append(buf, '\n'))
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
		}
		if len(authString) > 0 {
			upstreamConn.Write(authString)
		}
		upstreamConn.Write([]byte{'\n'})
		txtproto.R.Discard(txtproto.R.Buffered())
		bidirectionalPipe(ctx, conn, upstreamConn)
	}
}
func forwardConnHandler(dialer net.Dialer) connHandler {
	return func(ctx context.Context, conn io.ReadWriter) {
		start := time.Now()
		reader := bufio.NewReader(conn)
		firstLine, err := textproto.NewReader(reader).ReadLine()
		if err != nil {
			log.Printf("WARN: %v", err)
			return
		}
		tokens := strings.Split(firstLine, " ")
		if len(tokens) != 3 {
			return
		}
		defer func() {
			log.Printf("%s (%s)", firstLine, time.Since(start).String())
		}()
		switch tokens[0] {
		case "CONNECT":
			host := tokens[1]
			upstream, err := dialer.DialContext(ctx, "tcp", host)
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			defer upstream.Close()
			reader.Discard(reader.Buffered())
			_, err = conn.Write([]byte("HTTP/1.0 200 Connection established\n\n"))
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			bidirectionalPipe(ctx, conn, upstream)
		default:
			remoteURL, err := url.Parse(tokens[1])
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			port := remoteURL.Port()
			portNum := 0
			if port == "" {
				portNum = 80
			}
			host := remoteURL.Host
			if portNum != 0 {
				host = fmt.Sprintf("%s:%d", remoteURL.Host, portNum)
			}
			upstream, err := dialer.DialContext(ctx, "tcp", host)
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			defer upstream.Close()
			_, err = upstream.Write(append([]byte(firstLine), '\n'))
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			buf, err := reader.Peek(reader.Buffered())
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			upstream.Write(buf)
			bidirectionalPipe(ctx, conn, upstream)
		}
	}
}

type metricConn struct {
	conn         net.Conn
	writtenBytes int
	readBytes    int
}

func (m *metricConn) Write(buf []byte) (int, error) {
	n, err := m.conn.Write(buf)
	m.writtenBytes += n
	return n, err
}
func (m *metricConn) Read(buf []byte) (int, error) {
	n, err := m.conn.Read(buf)
	m.readBytes += n
	return n, err
}

func runHandler(handler connHandler, c net.Conn) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	defer c.Close()

	handler(ctx, &metricConn{conn: c})
}

func main() {
	config := viper.New()
	config.SetEnvPrefix("NANOPROXY")
	config.SetEnvKeyReplacer(strings.NewReplacer("-", "_"))
	config.AutomaticEnv()

	root := cobra.Command{
		Use: "nanoproxy",
		PreRun: func(cmd *cobra.Command, args []string) {
			config.BindEnv()
		},
		Run: func(cmd *cobra.Command, _ []string) {
			listener, err := net.Listen("tcp4", config.GetString("bind"))
			if err != nil {
				log.Fatal(err)
			}
			dialer := net.Dialer{KeepAlive: 15 * time.Second}
			upstreamURL := config.GetString("upstream")
			var h connHandler
			if upstreamURL != "" {
				h = upstreamConnHandler(dialer, config.GetString("upstream"))
			} else {
				h = forwardConnHandler(dialer)
			}
			var tempDelay time.Duration // how long to sleep on accept failure

			log.Printf("proxy listening on %s", listener.Addr().String())
			for {
				conn, err := listener.Accept()
				if err != nil {
					if ne, ok := err.(net.Error); ok && ne.Temporary() {
						if tempDelay == 0 {
							tempDelay = 5 * time.Millisecond
						} else {
							tempDelay *= 2
						}
						if max := 1 * time.Second; tempDelay > max {
							tempDelay = max
						}
						log.Printf("net/accept error: %v; retrying in %v", err, tempDelay)
						time.Sleep(tempDelay)
						continue
					}
					panic(err)
				}
				go runHandler(h, conn)
			}
		},
	}
	root.Flags().StringP("bind", "b", "0.0.0.0:8888", "bind to this address")
	root.Flags().StringP("upstream", "u", "", "forward requests to this proxy server")
	config.BindPFlag("bind", root.Flags().Lookup("bind"))
	config.BindPFlag("upstream", root.Flags().Lookup("upstream"))
	config.AutomaticEnv()
	err := root.Execute()
	if err != nil {
		log.Fatal(err)
	}
}
