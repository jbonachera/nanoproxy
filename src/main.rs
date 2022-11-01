mod dial;
mod pac;

use std::collections::HashMap;

use std::env;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;

use libproxy::ProxyFactory;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::TcpStream;

use dial::dial;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(version = env!("CARGO_PKG_VERSION"), author = env!("CARGO_PKG_AUTHORS"))]
pub struct Opts {
    /// Upstream proxy server
    #[clap(long, short = 'u')]
    upstream: Option<String>,

    /// listen on this network adress
    #[clap(long, short = 'b', default_value = "127.0.0.1:8889")]
    bind: String,
}

async fn parse_header_line(
    reader: &mut BufReader<TcpStream>,
) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
    let mut res = String::new();
    reader.read_line(&mut res).await?;
    let v = res.trim().splitn(2, ": ").collect::<Vec<&str>>();
    if v.len() != 2 {
        return Ok(None);
    }
    let key = v[0];
    let value = v[1];
    return Ok(Some((key.to_string(), value.to_string())));
}

async fn parse_headers(
    into: &mut HashMap<String, String>,
    reader: &mut BufReader<TcpStream>,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some((key, value)) = parse_header_line(reader).await? {
        into.insert(key.to_lowercase(), value);
    }
    Ok(())
}

async fn process(
    factory: ProxyFactory,
    socket: TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(socket);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).await?;
    let mut result = first_line.trim().splitn(3, " ");
    let method = result.next().unwrap().to_string();
    let path = result.next().unwrap().to_string();
    let version = result.next().unwrap();
    if version != "HTTP/1.1" {
        println!("dropping unsupported version {}", version);
        return Ok(());
    }
    let mut headers = HashMap::new();
    parse_headers(&mut headers, &mut reader).await?;

    let (remote, mut conn) = dial(factory, &method, &path, &mut headers).await?;

    let buf = reader.buffer();
    if buf.len() > 0 {
        conn.write(buf).await.map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to setup HTTP tunnel {remote}: {err}"),
            )
        })?;
    }
    let mut socket = reader.into_inner();

    if method == "CONNECT" {
        socket
            .write(format!("HTTP/1.1 200 Connection established\r\n\r\n").as_bytes())
            .await
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("failed to setup CONNECT tunnel to {remote}: {err}"),
                )
            })?;
    }
    let addr = conn.peer_addr()?;
    let (to, from) = tokio::io::copy_bidirectional(&mut socket, &mut conn)
        .await
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("bidir tunnel to {remote} failed: {err}"),
            )
        })?;
    println!("{remote} ({addr}): from={from} to={to}");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8889".to_string());

    println!("Listening on: {}", listen_addr);

    let listener = TcpListener::bind(listen_addr).await?;

    while let Ok((socket, addr)) = listener.accept().await {
        let factory = ProxyFactory::new().unwrap();

        tokio::spawn(async move {
            match process(factory, socket).await {
                Ok(_) => {}
                Err(v) => {
                    println!("tunnel failed: {}", v);
                }
            };
        });
    }
    Ok(())
}
