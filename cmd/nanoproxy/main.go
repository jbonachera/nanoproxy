package main

import (
	"log"
	"net"
	"net/http"
	"time"

	"github.com/jbonachera/nanoproxy/handler/httpproxy"
	"github.com/jbonachera/nanoproxy/handler/upstream"
	"github.com/spf13/cobra"

	"github.com/spf13/viper"
)

func main() {
	root := cobra.Command{
		Use: "nanoproxy",
		Run: func(cmd *cobra.Command, _ []string) {
			addr, err := net.ResolveTCPAddr("tcp", viper.GetString("bind"))
			if err != nil {
				log.Fatal(err)
			}
			listener, err := net.ListenTCP("tcp", addr)
			if err != nil {
				log.Fatal(err)
			}
			srv := &http.Server{
				ReadHeaderTimeout: 5 * time.Second,
				Addr:              addr.String(),
			}

			if upstreamStr := viper.GetString("upstream"); upstreamStr != "" {
				handler, err := upstream.NewHander(upstreamStr)
				if err != nil {
					log.Fatal(err)
				}
				srv.Handler = handler
			} else {
				handler, err := httpproxy.NewHander()
				if err != nil {
					log.Fatal(err)
				}
				srv.Handler = handler
			}
			log.Printf("proxy listening on %s", addr.String())
			log.Fatal(srv.Serve(listener))
		},
	}
	root.Flags().StringP("bind", "b", "0.0.0.0:8888", "bind to this address")
	root.Flags().StringP("upstream", "u", "", "forward requests to this proxy server")
	viper.BindPFlag("bind", root.Flags().Lookup("bind"))
	viper.BindPFlag("upstream", root.Flags().Lookup("upstream"))
	viper.AutomaticEnv()
	err := root.Execute()
	if err != nil {
		log.Fatal(err)
	}
}
