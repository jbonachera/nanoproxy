package main

import (
	"bufio"
	"encoding/base64"
	"fmt"
	"io"
	"log"
	"net"
	"net/textproto"
	"net/url"
	"strings"
	"time"

	"github.com/jbonachera/nanoproxy/requests"
	"github.com/spf13/cobra"

	"github.com/spf13/viper"
)

func upstreamConnHandler(dialer net.Dialer, upstreamURL string) func(conn net.Conn) {
	upstream, err := url.Parse(upstreamURL)
	if err != nil {
		panic(err)
	}
	return func(conn net.Conn) {
		defer conn.Close()
		upstreamConn, err := dialer.Dial("tcp4", upstream.Host)
		if err != nil {
			log.Printf("WARN: %v", err)
			return
		}
		defer upstreamConn.Close()
		txtproto := textproto.NewConn(conn)
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
		if user := upstream.User.String(); user != "" {
			auth := fmt.Sprintf("Basic %s", base64.StdEncoding.EncodeToString([]byte(upstream.User.String())))
			_, err = upstreamConn.Write([]byte(fmt.Sprintf("Proxy-Authorization: %s\n", auth)))
		}
		upstreamConn.Write([]byte{'\n'})
		go io.Copy(upstreamConn, conn)
		io.Copy(conn, upstreamConn)
	}
}
func connHandler(dialer net.Dialer) func(conn net.Conn) {
	return func(conn net.Conn) {
		defer conn.Close()
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
		log.Print(firstLine)

		switch tokens[0] {
		case "CONNECT":
			host := tokens[1]
			upstream, err := dialer.Dial("tcp4", host)
			if err != nil {
				log.Printf("WARN: %v", err)
				return
			}
			defer upstream.Close()
			conn.Write([]byte("HTTP/1.0 200 Connection established\n\n"))
			requests.ProcessConnect(conn, upstream)
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
			upstream, err := dialer.Dial("tcp4", host)
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
			conn.Write(buf)
			requests.ProcessConnect(conn, upstream)
		}
	}
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
			var h func(net.Conn)
			if upstreamURL != "" {
				h = upstreamConnHandler(dialer, config.GetString("upstream"))
			} else {
				h = connHandler(dialer)
			}
			log.Printf("proxy listening on %s", listener.Addr().String())
			for {
				conn, err := listener.Accept()
				if err != nil {
					log.Printf("net/accept failed: %v", err)
				}
				go h(conn)
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
