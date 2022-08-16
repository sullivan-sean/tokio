use crate::io::{Interest, PollEvented};
use crate::net::windows::{SocketAddr, UnixStream};

use std::convert::TryFrom;
use std::fmt;
use std::io;
use std::os::windows::io::{AsRawSocket, FromRawSocket, IntoRawSocket, RawSocket};
use mio::net::{stdnet as net};
use std::path::Path;
use std::task::{Context, Poll};

cfg_net_windows! {
    pub struct UnixListener {
        io: PollEvented<mio::net::UnixListener>,
    }
}

impl UnixListener {
    #[track_caller]
    pub fn bind<P>(path: P) -> io::Result<UnixListener>
    where
        P: AsRef<Path>,
    {
        let listener = mio::net::UnixListener::bind(path)?;
        let io = PollEvented::new(listener)?;
        Ok(UnixListener { io })
    }

    #[track_caller]
    pub fn from_std(listener: net::UnixListener) -> io::Result<UnixListener> {
        let listener = mio::net::UnixListener::from_std(listener);
        let io = PollEvented::new(listener)?;
        Ok(UnixListener { io })
    }

    pub fn into_std(self) -> io::Result<net::UnixListener> {
        self.io
            .into_inner()
            .map(|io| io.into_raw_socket())
            .map(|raw_socket| unsafe { net::UnixListener::from_raw_socket(raw_socket) })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr().map(SocketAddr)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.io.take_error()
    }

    pub async fn accept(&self) -> io::Result<(UnixStream, SocketAddr)> {
        let (mio, addr) = self
            .io
            .registration()
            .async_io(Interest::READABLE, || self.io.accept())
            .await?;

        let addr = SocketAddr(addr);
        let stream = UnixStream::new(mio)?;
        Ok((stream, addr))
    }

    pub fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<io::Result<(UnixStream, SocketAddr)>> {
        let (sock, addr) = ready!(self.io.registration().poll_read_io(cx, || self.io.accept()))?;
        let addr = SocketAddr(addr);
        let sock = UnixStream::new(sock)?;
        Poll::Ready(Ok((sock, addr)))
    }
}

impl TryFrom<net::UnixListener> for UnixListener {
    type Error = io::Error;

    fn try_from(stream: net::UnixListener) -> io::Result<Self> {
        Self::from_std(stream)
    }
}

impl fmt::Debug for UnixListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.io.fmt(f)
    }
}

impl AsRawSocket for UnixListener {
    fn as_raw_socket(&self) -> RawSocket {
        self.io.as_raw_socket()
    }
}
