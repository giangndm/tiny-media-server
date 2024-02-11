use std::{
    collections::VecDeque, io::{IoSlice, IoSliceMut}, mem::MaybeUninit, net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket}, os::fd::AsRawFd
};

use io_uring::{opcode, squeue, types, IoUring};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use super::UdpSocketGeneric;

#[derive(Debug, Clone)]
struct NetPacket {
    msg: MaybeUninit::<libc::msghdr>,
    buf: [u8; 1500],
    addr: socket2::SockAddr,
}

impl Default for NetPacket {
    fn default() -> Self {
        NetPacket {
            msg: MaybeUninit::uninit(),
            buf: [0; 1500],
            addr: SockAddr::from(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
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
    socket: Socket,
    local_addr: SocketAddr,
    send_bufs: Vec<NetPacket>,
    send_free_queue: VecDeque<usize>,
    send_queue_changed: bool,
    read_bufs: Vec<NetPacket>,
    read_wait_queue: VecDeque<usize>,
    read_queue_changed: bool,
}

impl<const QUEUE: usize> UdpSocket2IoUring<QUEUE> {
    pub fn new<T: ToSocketAddrs>(ip_addr: T) -> UdpSocket2IoUring<QUEUE> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).expect("Should create a socket");
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

        //prepare write
        for i in 0..QUEUE {
            let pkt = &mut send_bufs[i];
            unsafe {
                let p = pkt.msg.as_mut_ptr();
                (*p).msg_name = pkt.addr.as_ptr() as *const _ as *mut _;
                (*p).msg_namelen = pkt.addr.len();
                (*p).msg_iov =  IoSlice::new(&pkt.buf).as_ptr() as *mut _;
                (*p).msg_iovlen = 1;
            }
        }

        //prepare read
        for i in 0..QUEUE {
            let pkt = &mut read_bufs[i];
            unsafe {
                let p = pkt.msg.as_mut_ptr();
                (*p).msg_name = pkt.addr.as_ptr() as *const _ as *mut _;
                (*p).msg_namelen = pkt.addr.len();
                (*p).msg_iov = IoSliceMut::new(&mut pkt.buf).as_ptr() as *mut _;
                (*p).msg_iovlen = 1;
            }

            let recvmsg_e = opcode::RecvMsg::new(types::Fd(sockfd), pkt.msg.as_mut_ptr()).build()
                .user_data(UserData::Recv(i).into())
                .flags(squeue::Flags::ASYNC)
                .into();

            unsafe {
                ring
                    .submission().push(&recvmsg_e)
                    .expect("Should push recvmsg to io_uring");
            }
        }
        ring.submit().expect("Should submit recvmsg to io_uring");

        UdpSocket2IoUring {
            sockfd,
            ring,
            local_addr: socket.local_addr().expect("Should get local addr").as_socket().expect("Should be addr"),
            socket,
            read_bufs,
            send_bufs,
            send_queue_changed: false,
            read_queue_changed: false,
            send_free_queue: (0..QUEUE).collect(),
            read_wait_queue: VecDeque::new(),
        }
    }
}

impl<const QUEUE: usize> UdpSocketGeneric for UdpSocket2IoUring<QUEUE> {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&mut self, buf: &[u8], dest: SocketAddr) -> Result<usize, std::io::Error> {
        let pkt_idx = self.send_free_queue.pop_front().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "Send queue is full")
        })?;
        let pkt = &mut self.send_bufs[pkt_idx];
        pkt.addr = SockAddr::from(dest);
        pkt.buf[..buf.len()].copy_from_slice(buf);
        let p = pkt.msg.as_mut_ptr();
        unsafe {
            (*(*p).msg_iov).iov_len = buf.len();
        }

        let sendmsg_e = opcode::SendMsg::new(types::Fd(self.sockfd), pkt.msg.as_mut_ptr()).build()
            .user_data(UserData::Send(pkt_idx).into())
            .flags(squeue::Flags::ASYNC)
            .into();

        unsafe {
            self.ring
                .submission().push(&sendmsg_e)
                .expect("Should push sendmsg to io_uring");
        }

        self.send_queue_changed = true;
        
        Ok(buf.len())
    }

    fn commit_send_to(&mut self) -> Result<(), std::io::Error> {
        if self.send_queue_changed {
            self.ring.submit()?;
            self.send_queue_changed = false;
        }
        Ok(())
    }

    fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error> {
        while let Some(complete) = self.ring.completion().next() {
            match complete.user_data().into() {
                UserData::Recv(idx) => {
                    self.read_wait_queue.push_back(idx);
                }
                UserData::Send(idx) => {
                    self.send_free_queue.push_back(idx);
                }
            }
        }

        if let Some(idx) = self.read_wait_queue.pop_front() {
            let pkt = &mut self.read_bufs[idx];
            let len = pkt.buf.len();
            buf[..len].copy_from_slice(&pkt.buf[..len]);

            let recvmsg_e = opcode::RecvMsg::new(types::Fd(self.sockfd), pkt.msg.as_mut_ptr()).build()
                .user_data(UserData::Recv(idx).into())
                .flags(squeue::Flags::ASYNC)
                .into();

            unsafe {
                self.ring
                    .submission().push(&recvmsg_e)
                    .expect("Should push recvmsg to io_uring");
            }

            self.read_queue_changed = true;

            Ok((len, pkt.addr.as_socket().expect("Should be addr")))
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "No data to read"))
        }
    }

    fn finish_read_from(&mut self) -> Result<(), std::io::Error> {
        if self.read_queue_changed {
            self.ring.submit()?;
            self.read_queue_changed = false;
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
        let mut socket1 = UdpSocket2IoUring::<1024>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<1024>::new("127.0.0.1:0");

        socket1
            .add_send_to(&[1, 2, 3, 4], socket2.local_addr())
            .expect("Should ok");
        socket1.commit_send_to().expect("Should ok");
        std::thread::sleep(Duration::from_millis(100));

        let mut buf = [0; 1500];
        assert_eq!(
            socket2.recv_from(&mut buf).unwrap(),
            (4, socket1.local_addr())
        );
        assert_eq!(&buf[0..4], &[1, 2, 3, 4]);
        socket2.finish_read_from().expect("Should ok");
    }

    #[test]
    fn send_multi_msgs() {
        let mut socket1 = UdpSocket2IoUring::<1024>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<1024>::new("127.0.0.1:0");

        let addr1 = socket1.local_addr();
        let addr2 = socket2.local_addr();

        socket1.add_send_to(&[1], addr2).expect("Should ok");
        socket1.add_send_to(&[2], addr2).expect("Should ok");
        socket1.add_send_to(&[3], addr2).expect("Should ok");
        socket1.commit_send_to().expect("Should ok");
        std::thread::sleep(Duration::from_millis(100));

        for i in 1..=3 {
            let mut buf = [0; 1500];
            assert_eq!(socket2.recv_from(&mut buf).unwrap(), (1, addr1));
            assert_eq!(&buf[0..1], &[i]);
        }
        socket2.finish_read_from().expect("Should ok");
    }
}
