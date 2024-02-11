use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use socket2::{Domain, Socket, Type};

use super::UdpSocketGeneric;

pub struct UdpSocket2 {
    buf: [u8; 1500],
    socket: UdpSocket,
    local_addr: SocketAddr,
}

impl UdpSocket2 {
    pub fn new<T: ToSocketAddrs>(ip_addr: T) -> UdpSocket2 {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).expect("Should create a socket");
        let addrs = ip_addr.to_socket_addrs().unwrap();
        for addr in addrs {
            socket
                .bind(&addr.into())
                .expect("Should bind to a udp port");
        }
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
            buf: [0; 1500],
            local_addr: socket.local_addr().expect("Should get local addr"),
            socket,
        }
    }
}

impl UdpSocketGeneric for UdpSocket2 {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&mut self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error> {
        self.socket.send_to(buf, addr)
    }

    fn commit_send_to(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }

    fn recv_from(&mut self) -> Result<(&[u8], SocketAddr), std::io::Error> {
        let (size, remote) = self.socket.recv_from(&mut self.buf)?;
        Ok((&self.buf[0..size], remote))
    }

    fn finish_read_from(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}
