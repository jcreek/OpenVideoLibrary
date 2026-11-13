//! Cancel the underlying socket, including when Hyper is waiting for a slow
//! request body or blocked by a peer that stopped reading a response.
use axum::{
    extract::connect_info::Connected,
    serve::{IncomingStream, Listener},
};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

type Cancel = Pin<Box<dyn Future<Output = ()> + Send>>;
pub struct CancellableIo {
    inner: TcpStream,
    read_cancel: Cancel,
    write_cancel: Cancel,
}
impl CancellableIo {
    fn cancelled() -> io::Error {
        io::Error::new(io::ErrorKind::ConnectionAborted, "server stopping")
    }
}
impl AsyncRead for CancellableIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.read_cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Self::cancelled()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for CancellableIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Self::cancelled()));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Self::cancelled()));
        }
        Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.write_cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Self::cancelled()));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.write_cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

pub struct CancellableListener {
    inner: TcpListener,
    cancel: CancellationToken,
}
impl CancellableListener {
    pub fn new(inner: TcpListener, cancel: CancellationToken) -> Self {
        Self { inner, cancel }
    }
}
impl Listener for CancellableListener {
    type Io = CancellableIo;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((inner, addr)) => {
                    return (
                        CancellableIo {
                            inner,
                            read_cancel: Box::pin(self.cancel.clone().cancelled_owned()),
                            write_cancel: Box::pin(self.cancel.clone().cancelled_owned()),
                        },
                        addr,
                    );
                }
                Err(error) => {
                    tracing::warn!(%error,"connection admission failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PeerAddress(pub SocketAddr);
impl Connected<IncomingStream<'_, CancellableListener>> for PeerAddress {
    fn connect_info(stream: IncomingStream<'_, CancellableListener>) -> Self {
        Self(*stream.remote_addr())
    }
}
