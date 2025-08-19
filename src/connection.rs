use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::rt::{Read, Write};
use hyper_util::client::legacy::connect::{Connected, Connection};

use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

pub enum ProxyConnection<T> {
    Proxy { inner: T },
    NoProxy { inner: T },
}

impl<T: Read + Unpin> Read for ProxyConnection<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_read(cx, buf),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_read(cx, buf),
        }
    }
}

impl<T: Write + Unpin> Write for ProxyConnection<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_write(cx, buf),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_flush(cx),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_shutdown(cx),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_shutdown(cx),
        }
    }
}

impl<T> Connection for ProxyConnection<T> {
    fn connected(&self) -> Connected {
        match self {
            ProxyConnection::Proxy { inner: _ } => Connected::new().proxy(true),
            ProxyConnection::NoProxy { inner: _ } => Connected::new().proxy(false),
        }
    }
}

impl From<TokioIo<TcpStream>> for ProxyConnection<TokioIo<TcpStream>> {
    fn from(inner: TokioIo<TcpStream>) -> Self {
        ProxyConnection::Proxy { inner }
    }
}

impl ProxyConnection<TokioIo<TcpStream>> {
    pub fn into_proxy(self) -> Self {
        match self {
            ProxyConnection::Proxy { inner: _ } => self,
            ProxyConnection::NoProxy { inner } => ProxyConnection::Proxy { inner },
        }
    }
    pub fn into_direct(self) -> Self {
        match self {
            ProxyConnection::Proxy { inner } => ProxyConnection::NoProxy { inner },
            ProxyConnection::NoProxy { inner: _ } => self,
        }
    }
}
