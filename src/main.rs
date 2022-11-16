mod dial;
mod pac;

use std::collections::HashMap;

use actix::{
    Actor, ActorContext, ActorFutureExt, Addr, AsyncContext, Context, ContextFutureSpawner,
    Handler, Message, WrapFuture,
};
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

struct ProxyReq {
    version: String,
    method: String,
    resource: String,
    headers: HashMap<String, String>,
    reader: BufReader<TcpStream>,
}

async fn parse_req(socket: TcpStream) -> Result<ProxyReq, Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(socket);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).await?;
    let mut result = first_line.trim().splitn(3, " ");
    let method = result.next().unwrap().to_string();
    let resource = result.next().unwrap().to_string();
    let version = result.next().unwrap();

    let mut headers = HashMap::new();
    parse_headers(&mut headers, &mut reader).await?;
    Ok(ProxyReq {
        reader,
        headers,
        method,
        resource,
        version: version.to_string(),
    })
}

async fn process(socket: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let mut req = parse_req(socket).await?;
    let (remote, mut conn) = dial(&req.method, &req.resource, &mut req.headers).await?;

    let buf = req.reader.buffer();
    if buf.len() > 0 {
        conn.write(buf).await.map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to setup HTTP tunnel {remote}: {err}"),
            )
        })?;
    }
    let mut socket = req.reader.into_inner();

    if req.method == "CONNECT" {
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

#[derive(Message)]
#[rtype(result = "()")]
struct NewConnection {
    conn: TcpStream,
}
struct ProxyServer {}
impl Actor for ProxyServer {
    type Context = Context<Self>;
}

impl Handler<NewConnection> for ProxyServer {
    type Result = ();

    fn handle(&mut self, msg: NewConnection, ctx: &mut Self::Context) -> Self::Result {
        process(msg.conn)
            .into_actor(self)
            .then(|res, act, ctx| actix::fut::ready(()))
            .wait(ctx);
    }
}

#[actix::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8889".to_string());

    println!("Listening on: {}", listen_addr);

    let server = ProxyServer::create(|ctx| ProxyServer {});

    let listener = TcpListener::bind(listen_addr).await?;

    while let Ok((conn, addr)) = listener.accept().await {
        server.do_send(NewConnection { conn });
    }
    Ok(())
}
