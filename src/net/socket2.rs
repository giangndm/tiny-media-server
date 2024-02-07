use std::net::{IpAddr, SocketAddr, UdpSocket};

use socket2::{Domain, Socket, Type};

use super::UdpSocketGeneric;

pub struct UdpSocket2 {
    socket: UdpSocket,
    local_addr: SocketAddr,
}

impl UdpSocket2 {
    pub fn new(ip_addr: IpAddr) -> UdpSocket2 {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).expect("Should create a socket");
        socket
            .bind(&SocketAddr::new(ip_addr, 0).into())
            .expect("Should bind to a udp port");
        socket
            .set_send_buffer_size(1024 * 1024)
            .expect("Should set send buffer size");
        socket
            .set_recv_buffer_size(1024 * 1024)
            .expect("Should set recv buffer size");
        socket
            .set_nonblocking(true)
            .expect("Should set nonblocking");
        let socket: UdpSocket = socket.into();

        UdpSocket2 {
            local_addr: socket.local_addr().expect("Should get local addr"),
            socket,
        }
    }
}

impl UdpSocketGeneric for UdpSocket2 {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error> {
        self.socket.send_to(buf, addr)
    }

    fn commit_send_to(&self) -> Result<(), std::io::Error> {
        Ok(())
    }

    fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error> {
        self.socket.recv_from(buf)
    }
}
