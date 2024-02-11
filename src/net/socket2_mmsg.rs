use std::{
    io::IoSlice,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    os::fd::AsRawFd,
};

use nix::sys::socket::{sendmmsg, MsgFlags, MultiHeaders, SockaddrStorage};
use socket2::{Domain, Socket, Type};

use super::UdpSocketGeneric;

pub struct UdpSocket2Mmsg<const QUEUE: usize> {
    sockfd: i32,
    socket: UdpSocket,
    local_addr: SocketAddr,
    data: MultiHeaders<SockaddrStorage>,
    bufs: [([u8; 1500], usize); QUEUE],
    addrs: [Option<SockaddrStorage>; QUEUE],
    queue_len: usize,
    recv_buf: [u8; 1500],
}

impl<const QUEUE: usize> UdpSocket2Mmsg<QUEUE> {
    pub fn new<T: ToSocketAddrs>(ip_addr: T) -> UdpSocket2Mmsg<QUEUE> {
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

        UdpSocket2Mmsg {
            sockfd: socket.as_raw_fd(),
            local_addr: socket.local_addr().expect("Should get local addr"),
            socket,
            data: MultiHeaders::preallocate(QUEUE, None),
            bufs: [([0; 1500], 0); QUEUE],
            addrs: [None; QUEUE],
            queue_len: 0,
            recv_buf: [0; 1500],
        }
    }
}

impl<const QUEUE: usize> UdpSocketGeneric for UdpSocket2Mmsg<QUEUE> {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&mut self, buf: &[u8], dest: SocketAddr) -> Result<usize, std::io::Error> {
        if self.queue_len == QUEUE {
            self.commit_send_to()?;
            self.queue_len = 0;
        }

        let slot = self.queue_len;
        self.queue_len += 1;

        self.bufs[slot].1 = buf.len();
        self.bufs[slot].0[0..buf.len()].copy_from_slice(buf);
        self.addrs[slot] = Some(dest.into());

        Ok(buf.len())
    }

    fn commit_send_to(&mut self) -> Result<(), std::io::Error> {
        let mut iovs = vec![];
        for i in 0..self.queue_len {
            let (buf, len) = &self.bufs[i];
            iovs.push([IoSlice::new(&buf[0..*len])]);
        }

        sendmmsg(
            self.sockfd,
            &mut self.data,
            &iovs,
            &self.addrs[0..self.queue_len],
            [],
            MsgFlags::empty(),
        )?;

        Ok(())
    }

    fn recv_from(&mut self) -> Result<(&[u8], SocketAddr), std::io::Error> {
        let (size, remote) = self.socket.recv_from(&mut self.recv_buf)?;
        Ok((&self.recv_buf[0..size], remote))
    }

    fn finish_read_from(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::net::UdpSocketGeneric;

    use super::UdpSocket2Mmsg;

    #[test]
    fn send_single_msg() {
        let mut socket1 = UdpSocket2Mmsg::<2>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2Mmsg::<2>::new("127.0.0.1:0");

        let buf = vec![1, 2, 3, 4];

        socket1
            .add_send_to(&buf, socket2.local_addr())
            .expect("Should ok");
        socket1.commit_send_to().expect("Should ok");
        std::thread::sleep(Duration::from_millis(100));

        assert_eq!(
            socket2.recv_from().unwrap(),
            (buf.as_slice(), socket1.local_addr())
        );
    }

    #[test]
    fn send_multi_msgs() {
        let mut socket1 = UdpSocket2Mmsg::<2>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2Mmsg::<2>::new("127.0.0.1:0");

        let addr1 = socket1.local_addr();
        let addr2 = socket2.local_addr();

        socket1.add_send_to(&[1], addr2).expect("Should ok");
        socket1.add_send_to(&[2], addr2).expect("Should ok");
        socket1.add_send_to(&[3], addr2).expect("Should ok");
        socket1.commit_send_to().expect("Should ok");
        std::thread::sleep(Duration::from_millis(100));

        for i in 1..=3 {
            assert_eq!(socket2.recv_from().unwrap(), (vec![i].as_slice(), addr1));
        }
    }
}
