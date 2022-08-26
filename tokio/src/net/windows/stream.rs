use crate::future::poll_fn;
use crate::io::{AsyncRead, AsyncWrite, Interest, PollEvented, ReadBuf, Ready};
use crate::net::windows::split::{split, ReadHalf, WriteHalf};
use crate::net::windows::split_owned::{split_owned, OwnedReadHalf, OwnedWriteHalf};
use crate::net::windows::SocketAddr;

use std::fmt;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

cfg_io_util! {
    use bytes::BufMut;
}

cfg_net_windows! {
    pub struct UnixStream {
        io: PollEvented<mio::net::UnixStream>,
    }
}

impl UnixStream {
    pub async fn connect<P>(path: P) -> io::Result<UnixStream>
    where
        P: AsRef<Path>,
    {
        let stream = mio::net::UnixStream::connect(path)?;
        let stream = UnixStream::new(stream)?;

        poll_fn(|cx| stream.io.registration().poll_write_ready(cx)).await?;

        if let Some(e) = stream.io.take_error()? {
            return Err(e);
        }

        Ok(stream)
    }

    pub async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        let event = self.io.registration().readiness(interest).await?;
        Ok(event.ready)
    }

    pub async fn readable(&self) -> io::Result<()> {
        self.ready(Interest::READABLE).await?;
        Ok(())
    }

    pub fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.io.registration().poll_read_ready(cx).map_ok(|_| ())
    }

    pub fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.io
            .registration()
            .try_io(Interest::READABLE, || (&*self.io).read(buf))
    }

    pub fn try_read_vectored(&self, bufs: &mut [io::IoSliceMut<'_>]) -> io::Result<usize> {
        self.io
            .registration()
            .try_io(Interest::READABLE, || (&*self.io).read_vectored(bufs))
    }

    cfg_io_util! {
        pub fn try_read_buf<B: BufMut>(&self, buf: &mut B) -> io::Result<usize> {
            self.io.registration().try_io(Interest::READABLE, || {
                use std::io::Read;

                let dst = buf.chunk_mut();
                let dst =
                    unsafe { &mut *(dst as *mut _ as *mut [std::mem::MaybeUninit<u8>] as *mut [u8]) };

                // Safety: We trust `UnixStream::read` to have filled up `n` bytes in the
                // buffer.
                let n = (&*self.io).read(dst)?;

                unsafe {
                    buf.advance_mut(n);
                }

                Ok(n)
            })
        }
    }

    pub async fn writable(&self) -> io::Result<()> {
        self.ready(Interest::WRITABLE).await?;
        Ok(())
    }

    pub fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.io.registration().poll_write_ready(cx).map_ok(|_| ())
    }

    pub fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        self.io
            .registration()
            .try_io(Interest::WRITABLE, || (&*self.io).write(buf))
    }

    pub fn try_write_vectored(&self, buf: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.io
            .registration()
            .try_io(Interest::WRITABLE, || (&*self.io).write_vectored(buf))
    }

    pub fn try_io<R>(
        &self,
        interest: Interest,
        f: impl FnOnce() -> io::Result<R>,
    ) -> io::Result<R> {
        self.io
            .registration()
            .try_io(interest, || self.io.try_io(f))
    }

    pub fn pair() -> io::Result<(UnixStream, UnixStream)> {
        let (a, b) = mio::net::UnixStream::pair()?;
        let a = UnixStream::new(a)?;
        let b = UnixStream::new(b)?;

        Ok((a, b))
    }

    pub(crate) fn new(stream: mio::net::UnixStream) -> io::Result<UnixStream> {
        let io = PollEvented::new(stream)?;
        Ok(UnixStream { io })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr().map(SocketAddr)
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.io.peer_addr().map(SocketAddr)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.io.take_error()
    }

    pub(super) fn shutdown_std(&self, how: Shutdown) -> io::Result<()> {
        self.io.shutdown(how)
    }

    // These lifetime markers also appear in the generated documentation, and make
    // it more clear that this is a *borrowed* split.
    #[allow(clippy::needless_lifetimes)]
    pub fn split<'a>(&'a mut self) -> (ReadHalf<'a>, WriteHalf<'a>) {
        split(self)
    }

    pub fn into_split(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        split_owned(self)
    }
}

impl AsyncRead for UnixStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.poll_read_priv(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.poll_write_priv(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.poll_write_vectored_priv(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.shutdown_std(std::net::Shutdown::Write)?;
        Poll::Ready(Ok(()))
    }
}

impl UnixStream {
    // == Poll IO functions that takes `&self` ==
    //
    // To read or write without mutable access to the `UnixStream`, combine the
    // `poll_read_ready` or `poll_write_ready` methods with the `try_read` or
    // `try_write` methods.

    pub(crate) fn poll_read_priv(
        &self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // Safety: `UnixStream::read` correctly handles reads into uninitialized memory
        unsafe { self.io.poll_read(cx, buf) }
    }

    pub(crate) fn poll_write_priv(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.io.poll_write(cx, buf)
    }

    pub(super) fn poll_write_vectored_priv(
        &self,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.io.poll_write_vectored(cx, bufs)
    }
}

impl fmt::Debug for UnixStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.io.fmt(f)
    }
}

impl AsRawSocket for UnixStream {
    fn as_raw_socket(&self) -> RawSocket {
        self.io.as_raw_socket()
    }
}
