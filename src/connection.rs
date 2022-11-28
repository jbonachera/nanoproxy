use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::client::connect::Connection;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use tokio::net::TcpStream;
pub enum ProxyConnection<T> {
    Proxy { inner: T },
    NoProxy { inner: T },
}

impl<T: AsyncRead + Unpin> AsyncRead for ProxyConnection<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<tokio::io::Result<()>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_read(cx, buf),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_read(cx, buf),
        }
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for ProxyConnection<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<core::result::Result<usize, tokio::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_write(cx, buf),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<core::result::Result<(), std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_flush(cx),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<core::result::Result<(), std::io::Error>> {
        match &mut *self {
            ProxyConnection::Proxy { inner } => Pin::new(inner).poll_shutdown(cx),
            ProxyConnection::NoProxy { inner } => Pin::new(inner).poll_shutdown(cx),
        }
    }
}
impl<T> Connection for ProxyConnection<T> {
    fn connected(&self) -> hyper::client::connect::Connected {
        match self {
            ProxyConnection::Proxy { inner: _ } => {
                hyper::client::connect::Connected::new().proxy(true)
            }
            ProxyConnection::NoProxy { inner: _ } => {
                hyper::client::connect::Connected::new().proxy(false)
            }
        }
    }
}

impl From<TcpStream> for ProxyConnection<TcpStream> {
    fn from(inner: TcpStream) -> Self {
        ProxyConnection::Proxy { inner }
    }
}

impl ProxyConnection<TcpStream> {
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
