use std::fmt;
use std::path::Path;

pub struct SocketAddr(pub(super) mio::net::SocketAddr);

impl SocketAddr {
    pub fn is_unnamed(&self) -> bool {
        self.0.is_unnamed()
    }

    pub fn as_pathname(&self) -> Option<&Path> {
        self.0.as_pathname()
    }
}

impl fmt::Debug for SocketAddr {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(fmt)
    }
}
