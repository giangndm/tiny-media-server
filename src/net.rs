use std::net::SocketAddr;

pub mod socket2;

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
))]
pub mod socket2_mmsg;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod socket2_io_uring;

pub trait UdpSocketGeneric {
    fn prepare(&mut self);
    fn local_addr(&self) -> SocketAddr;
    fn add_send_to(&mut self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error>;
    fn commit_send_to(&mut self) -> Result<(), std::io::Error>;
    fn recv_from(&mut self) -> Result<(&[u8], SocketAddr), std::io::Error>;
    fn finish_read_from(&mut self) -> Result<(), std::io::Error>;
}
