use std::net::SocketAddr;

pub mod socket2;

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
))]
pub mod socket2_mmsg;

pub trait UdpSocketGeneric {
    fn local_addr(&self) -> SocketAddr;
    fn add_send_to(&mut self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error>;
    fn commit_send_to(&mut self) -> Result<(), std::io::Error>;
    fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error>;
}
