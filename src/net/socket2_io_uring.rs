use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    os::fd::AsRawFd,
};

use io_uring::{opcode, squeue, types, IoUring};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use super::UdpSocketGeneric;

#[derive(Debug, Clone)]
struct NetPacket {
    msg: libc::msghdr,
    buf: [u8; 1500],
    addr: socket2::SockAddr,
    iovecs: [libc::iovec; 1],
}

impl Default for NetPacket {
    fn default() -> Self {
        NetPacket {
            msg: unsafe { std::mem::zeroed() },
            buf: [0; 1500],
            addr: SockAddr::from(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
            iovecs: [libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }],
        }
    }
}

enum UserData {
    Send(usize),
    Recv(usize),
}

impl Into<u64> for UserData {
    fn into(self) -> u64 {
        match self {
            UserData::Send(idx) => idx as u64,
            UserData::Recv(idx) => 0x01_00000000 | (idx as u64),
        }
    }
}

impl From<u64> for UserData {
    fn from(data: u64) -> UserData {
        if data & 0x01_00000000 == 0 {
            UserData::Send(data as usize)
        } else {
            UserData::Recv((data & 0x00_FF_FF_FF_FF) as usize)
        }
    }
}

pub struct UdpSocket2IoUring<const QUEUE: usize> {
    ring: IoUring,
    sockfd: i32,
    _socket: Socket,
    local_addr: SocketAddr,
    send_bufs: Vec<NetPacket>,
    send_free_queue: VecDeque<usize>,
    read_bufs: Vec<NetPacket>,
    read_wait_queue: VecDeque<(usize, usize)>,
    ring_queue_changed: bool,
}

impl<const QUEUE: usize> UdpSocket2IoUring<QUEUE> {
    pub fn new<T: ToSocketAddrs>(ip_addr: T) -> UdpSocket2IoUring<QUEUE> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .expect("Should create a socket");
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
        let sockfd = socket.as_raw_fd();

        let mut ring = IoUring::new(QUEUE as u32 * 2).expect("Should create io_uring");
        let mut send_bufs = Vec::with_capacity(QUEUE);
        let mut read_bufs = Vec::with_capacity(QUEUE);
        for _ in 0..QUEUE {
            send_bufs.push(NetPacket::default());
            read_bufs.push(NetPacket::default());
        }

        // preapre write
        for i in 0..QUEUE {
            let pkt = &mut send_bufs[i];

            pkt.iovecs[0].iov_base = pkt.buf.as_mut_ptr() as *mut _;
            pkt.iovecs[0].iov_len = pkt.buf.len();

            pkt.msg.msg_name = pkt.addr.as_ptr() as *const _ as *mut _;
            pkt.msg.msg_namelen = pkt.addr.len();
            pkt.msg.msg_iov = &mut pkt.iovecs as *mut _;
            pkt.msg.msg_iovlen = 1;
        }

        // prepare read
        for i in 0..QUEUE {
            let pkt = &mut read_bufs[i];

            pkt.iovecs[0].iov_base = pkt.buf.as_mut_ptr() as *mut _;
            pkt.iovecs[0].iov_len = pkt.buf.len();

            pkt.msg.msg_name = pkt.addr.as_ptr() as *const _ as *mut _;
            pkt.msg.msg_namelen = pkt.addr.len();
            pkt.msg.msg_iov = &mut pkt.iovecs as *mut _;
            pkt.msg.msg_iovlen = 1;

            let recvmsg_e = opcode::RecvMsg::new(types::Fd(sockfd), &mut pkt.msg)
                .build()
                .user_data(UserData::Recv(i).into())
                .flags(squeue::Flags::ASYNC)
                .into();

            unsafe {
                ring.submission()
                    .push(&recvmsg_e)
                    .expect("Should push recvmsg to io_uring");
            }
        }
        ring.submit().expect("Should submit recvmsg to io_uring");

        UdpSocket2IoUring {
            sockfd,
            ring,
            local_addr: socket
                .local_addr()
                .expect("Should get local addr")
                .as_socket()
                .expect("Should be addr"),
            _socket: socket,
            read_bufs,
            send_bufs,
            send_free_queue: (0..QUEUE).collect(),
            read_wait_queue: VecDeque::new(),
            ring_queue_changed: false,
        }
    }

    fn poll_completions(&mut self) {
        while let Some(complete) = self.ring.completion().next() {
            if complete.result() > 0 {
                match complete.user_data().into() {
                    UserData::Recv(idx) => {
                        self.read_wait_queue
                            .push_back((idx, complete.result() as usize));
                    }
                    UserData::Send(idx) => {
                        self.send_free_queue.push_back(idx);
                    }
                }
            } else {
                log::error!("Error: {}", complete.result());
            }
        }
    }
}

impl<const QUEUE: usize> UdpSocketGeneric for UdpSocket2IoUring<QUEUE> {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&mut self, buf: &[u8], dest: SocketAddr) -> Result<usize, std::io::Error> {
        self.poll_completions();
        let pkt_idx = self.send_free_queue.pop_front().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "Send queue is full")
        })?;
        let pkt = &mut self.send_bufs[pkt_idx];
        pkt.addr = SockAddr::from(dest);
        pkt.buf[..buf.len()].copy_from_slice(buf);
        pkt.iovecs[0].iov_len = buf.len();
        pkt.msg.msg_namelen = pkt.addr.len();

        let sendmsg_e = opcode::SendMsg::new(types::Fd(self.sockfd), &pkt.msg)
            .build()
            .user_data(UserData::Send(pkt_idx).into())
            .flags(squeue::Flags::IO_LINK)
            .into();

        unsafe {
            self.ring
                .submission()
                .push(&sendmsg_e)
                .expect("Should push sendmsg to io_uring");
        }

        self.ring_queue_changed = true;

        Ok(buf.len())
    }

    fn commit_send_to(&mut self) -> Result<(), std::io::Error> {
        if self.ring_queue_changed {
            self.ring.submit()?;
            self.ring_queue_changed = false;
        }
        Ok(())
    }

    fn recv_from(&mut self) -> Result<(&[u8], SocketAddr), std::io::Error> {
        self.poll_completions();
        if let Some((idx, len)) = self.read_wait_queue.pop_front() {
            let pkt = &mut self.read_bufs[idx];

            let recvmsg_e = opcode::RecvMsg::new(types::Fd(self.sockfd), &mut pkt.msg)
                .build()
                .user_data(UserData::Recv(idx).into())
                .flags(squeue::Flags::ASYNC)
                .into();

            unsafe {
                self.ring
                    .submission()
                    .push(&recvmsg_e)
                    .expect("Should push recvmsg to io_uring");
            }

            self.ring_queue_changed = true;

            Ok((
                &pkt.buf[..len],
                pkt.addr.as_socket().expect("Should be addr"),
            ))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "No data to read",
            ))
        }
    }

    fn finish_read_from(&mut self) -> Result<(), std::io::Error> {
        if self.ring_queue_changed {
            self.ring.submit()?;
            self.ring_queue_changed = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::net::UdpSocketGeneric;

    use super::UdpSocket2IoUring;

    #[test]
    fn send_single_msg() {
        let mut socket1 = UdpSocket2IoUring::<8>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<8>::new("127.0.0.1:0");

        socket1
            .add_send_to(&[1, 2, 3, 4], socket2.local_addr())
            .expect("Should ok");
        socket1.commit_send_to().expect("Should ok");
        std::thread::sleep(Duration::from_millis(100));

        assert_eq!(
            socket2.recv_from().unwrap(),
            (vec![1, 2, 3, 4].as_slice(), socket1.local_addr())
        );
        socket2.finish_read_from().expect("Should ok");
    }

    #[test]
    fn send_multi_msgs() {
        let mut socket1 = UdpSocket2IoUring::<4>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<4>::new("127.0.0.1:0");

        let addr1 = socket1.local_addr();
        let addr2 = socket2.local_addr();

        for _ in 0..3 {
            socket1.add_send_to(&[1], addr2).expect("Should ok");
            socket1.add_send_to(&[2], addr2).expect("Should ok");
            socket1.add_send_to(&[3], addr2).expect("Should ok");
            socket1.commit_send_to().expect("Should ok");
            std::thread::sleep(Duration::from_millis(100));

            for i in 1..=3 {
                assert_eq!(socket2.recv_from().unwrap(), (vec![i].as_slice(), addr1));
            }
            socket2.finish_read_from().expect("Should ok");
        }
    }
}
