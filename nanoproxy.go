package main

import (
	"bufio"
	"context"
	"encoding/base64"
	"errors"
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

func upstreamProxyResolver(dialer net.Dialer, upstreamURL string) upstreamResolver {
	upstream, err := url.Parse(upstreamURL)
	if err != nil {
		panic(err)
	}
	authString := []byte{}
	if user := upstream.User.String(); user != "" {
		auth := fmt.Sprintf("Basic %s", base64.StdEncoding.EncodeToString([]byte(upstream.User.String())))
		authString = []byte(fmt.Sprintf("Proxy-Authorization: %s\n", auth))
	}
	return func(ctx context.Context, conn io.ReadWriter) (net.Conn, string, error) {
		upstreamConn, err := dialer.DialContext(ctx, "tcp", upstream.Host)
		if err != nil {
			return nil, "", err
		}
		reader := bufio.NewReader(conn)
		txtproto := textproto.NewReader(reader)
		first := true
		remoteHost := ""
		for {
			buf, err := txtproto.ReadLineBytes()
			if first {
				tokens := strings.Split(string(buf), " ")
				if len(tokens) == 3 {
					return nil, "", errors.New("malformed request")
				}
				remoteHost = tokens[1]
				first = false
			}
			if err != nil {
				upstreamConn.Close()
				return nil, "", err
			}
			if len(buf) == 0 {
				break
			}
			_, err = upstreamConn.Write(append(buf, '\n'))
			if err != nil {
				upstreamConn.Close()
				return nil, "", err
			}
		}
		if len(authString) > 0 {
			upstreamConn.Write(authString)
		}
		upstreamConn.Write([]byte{'\n'})
		txtproto.R.Discard(txtproto.R.Buffered())
		return upstreamConn, remoteHost, nil
	}
}

type upstreamResolver func(ctx context.Context, conn io.ReadWriter) (upstream net.Conn, host string, err error)

func staticUpstreamResolver(dialer net.Dialer) upstreamResolver {
	return func(ctx context.Context, conn io.ReadWriter) (net.Conn, string, error) {
		reader := bufio.NewReader(conn)
		firstLine, err := textproto.NewReader(reader).ReadLine()
		if err != nil {
			return nil, "", err
		}
		tokens := strings.Split(firstLine, " ")
		if len(tokens) != 3 {
			return nil, "", errors.New("malformed http request")
		}
		switch tokens[0] {
		case "CONNECT":
			host := tokens[1]
			upstream, err := dialer.DialContext(ctx, "tcp", host)
			if err != nil {
				return nil, "", err
			}
			reader.Discard(reader.Buffered())
			_, err = conn.Write([]byte("HTTP/1.0 200 Connection established\n\n"))
			if err != nil {
				upstream.Close()
				return nil, "", err
			}
			return upstream, host, nil
		default:
			remoteURL, err := url.Parse(tokens[1])
			if err != nil {
				return nil, "", err
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
				return nil, "", err
			}
			_, err = upstream.Write(append([]byte(firstLine), '\n'))
			if err != nil {
				upstream.Close()
				return nil, "", err
			}
			buf, err := reader.Peek(reader.Buffered())
			if err != nil {
				upstream.Close()
				return nil, "", err
			}
			upstream.Write(buf)
			return upstream, host, nil
		}
	}
}

type metricConn struct {
	conn         net.Conn
	host         string
	startedAt    time.Time
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

func runHandler(stats chan event, resolver upstreamResolver, c net.Conn) {
	start := time.Now()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	defer c.Close()
	local := &metricConn{conn: c, startedAt: start}
	remote, host, err := resolver(ctx, local)
	local.host = host
	if err != nil {
		log.Printf("WARN: %v", err)
		return
	}
	defer remote.Close()
	stats <- event{kind: connAdded, conn: local}
	bidirectionalPipe(ctx, local, remote)
	stats <- event{kind: connRemoved, conn: local}
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
			var h upstreamResolver
			if upstreamURL != "" {
				h = upstreamProxyResolver(dialer, config.GetString("upstream"))
			} else {
				h = staticUpstreamResolver(dialer)
			}
			var tempDelay time.Duration // how long to sleep on accept failure

			log.Printf("proxy listening on %s", listener.Addr().String())
			stats := runStats()
			defer close(stats)
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
				go runHandler(stats, h, conn)
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
