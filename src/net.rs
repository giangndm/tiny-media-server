use std::net::SocketAddr;

pub mod socket2;

pub trait UdpSocketGeneric {
    fn local_addr(&self) -> SocketAddr;
    fn add_send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error>;
    fn commit_send_to(&self) -> Result<(), std::io::Error>;
    fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error>;
}
