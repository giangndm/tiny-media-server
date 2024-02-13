use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    os::fd::AsRawFd,
};

use io_uring::{opcode, types, IoUring, Probe};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use super::UdpSocketGeneric;

const RECV_BUF_SIZE: usize = 1532;
const RECV_BUF_GROUP_SIZE: u32 = 16;

struct GroupIndex;

impl GroupIndex {
    fn to_u32(group: u16, index: u16) -> u32 {
        group as u32 * RECV_BUF_GROUP_SIZE + index as u32
    }

    // fn from_u32(idx: u32) -> (u16, u16) {
    //     ((idx / RECV_BUF_GROUP_SIZE) as u16, (idx % RECV_BUF_GROUP_SIZE) as u16)
    // }

    // fn group(idx: u32) -> u16 {
    //     (idx / RECV_BUF_GROUP_SIZE) as u16
    // }

    fn index(idx: u32) -> u16 {
        (idx % RECV_BUF_GROUP_SIZE) as u16
    }
}

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

#[derive(Debug)]
enum UserData {
    Send(u32),
    Recv(u16),
    ProvideBuffers(u32),
}

impl Into<u64> for UserData {
    fn into(self) -> u64 {
        match self {
            UserData::Send(idx) => idx as u64,
            UserData::Recv(idx) => 0x01_00000000 | (idx as u64),
            UserData::ProvideBuffers(idx) => 0x02_00000000 | (idx as u64),
        }
    }
}

impl From<u64> for UserData {
    fn from(data: u64) -> UserData {
        match data >> 32 {
            0x01 => UserData::Recv((data & 0x00_0000ffff) as u16),
            0x02 => UserData::ProvideBuffers((data & 0x00_ffffffff) as u32),
            _ => UserData::Send((data & 0x00_ffffffff) as u32),
        }
    }
}

pub struct UdpSocket2IoUring<const SEND_QUEUE: usize, const RECV_QUEUE: usize> {
    ring: IoUring,
    sockfd: i32,
    _socket: socket2::Socket,
    local_addr: SocketAddr,
    send_bufs: Vec<NetPacket>,
    send_free_queue: VecDeque<u32>,
    read_bufs: [[u8; RECV_BUF_SIZE]; RECV_QUEUE],
    read_wait_queue: VecDeque<u32>,
    read_wait_free_groups: VecDeque<u16>,
    ring_queue_changed: bool,
    group_msghdr: libc::msghdr,
}

impl<const SEND_QUEUE: usize, const RECV_QUEUE: usize> UdpSocket2IoUring<SEND_QUEUE, RECV_QUEUE> {
    pub fn new<T: ToSocketAddrs>(ip_addr: T) -> UdpSocket2IoUring<SEND_QUEUE, RECV_QUEUE> {
        assert_eq!(
            RECV_QUEUE % RECV_BUF_GROUP_SIZE as usize,
            0,
            "RECV_QUEUE should be multiple of {RECV_BUF_GROUP_SIZE}"
        );

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
        socket
            .set_nonblocking(true)
            .expect("Should set nonblocking");
        let sockfd = socket.as_raw_fd();

        let ring = IoUring::new((SEND_QUEUE + RECV_QUEUE) as u32).expect("Should create io_uring");

        let mut probe = Probe::new();
        if ring.submitter().register_probe(&mut probe).is_err() {
            eprintln!("No probe supported");
        }

        assert_eq!(probe.is_supported(opcode::RecvMsgMulti::CODE), true);
        assert_eq!(probe.is_supported(opcode::ProvideBuffers::CODE), true);
        assert_eq!(probe.is_supported(opcode::SendMsgZc::CODE), true);
        assert_eq!(probe.is_supported(opcode::SendMsg::CODE), true);

        let mut send_bufs = Vec::with_capacity(SEND_QUEUE);
        for _ in 0..SEND_QUEUE {
            send_bufs.push(NetPacket::default());
        }

        UdpSocket2IoUring {
            sockfd,
            ring,
            local_addr: socket
                .local_addr()
                .expect("Should get local addr")
                .as_socket()
                .expect("Should be addr"),
            _socket: socket.into(),
            read_bufs: [[0 as u8; RECV_BUF_SIZE]; RECV_QUEUE],
            send_bufs,
            send_free_queue: (0..SEND_QUEUE as u32).collect(),
            read_wait_queue: VecDeque::new(),
            read_wait_free_groups: VecDeque::new(),
            ring_queue_changed: false,
            group_msghdr: unsafe { std::mem::zeroed() },
        }
    }

    fn preapre_write(&mut self) {
        for i in 0..SEND_QUEUE {
            let pkt = &mut self.send_bufs[i];

            pkt.iovecs[0].iov_base = pkt.buf.as_mut_ptr() as *mut _;
            pkt.iovecs[0].iov_len = pkt.buf.len();

            pkt.msg.msg_name = pkt.addr.as_ptr() as *const _ as *mut _;
            pkt.msg.msg_namelen = pkt.addr.len();
            pkt.msg.msg_iov = &mut pkt.iovecs as *mut _;
            pkt.msg.msg_iovlen = 1;
        }
    }

    fn prepare_read(&mut self, group: u16) {
        let begin_idx = GroupIndex::to_u32(group, 0);
        let end_idx = GroupIndex::to_u32(group + 1, 0);

        for index in begin_idx..end_idx {
            let buf = &mut self.read_bufs[index as usize];
            let provide_bufs_e = io_uring::opcode::ProvideBuffers::new(
                buf.as_mut_ptr(),
                RECV_BUF_SIZE as i32,
                1,
                group,
                GroupIndex::index(index),
            )
            .build()
            .user_data(UserData::ProvideBuffers(index as u32).into())
            .into();
            unsafe {
                self.ring
                    .submission()
                    .push(&provide_bufs_e)
                    .expect("Should submit provide bufs")
            };
        }

        // This structure is actually only used for input arguments to the kernel
        // (and only name length and control length are actually relevant).
        self.group_msghdr.msg_namelen = 32;
        self.group_msghdr.msg_controllen = 0;

        let recvmsg_e = opcode::RecvMsgMulti::new(
            types::Fd(self.sockfd),
            &self.group_msghdr as *const _,
            group,
        )
        .build()
        .user_data(UserData::Recv(group).into())
        .into();
        unsafe {
            self.ring
                .submission()
                .push(&recvmsg_e)
                .expect("Should submit")
        };

        self.ring.submit().expect("Should submit");
    }

    fn poll_completions(&mut self) {
        while let Some(complete) = self.ring.completion().next() {
            let is_more = io_uring::cqueue::more(complete.flags());
            let user_data = complete.user_data().into();
            match user_data {
                UserData::Recv(group) => {
                    if is_more {
                        let idx = io_uring::cqueue::buffer_select(complete.flags())
                            .expect("Should select buffer id");
                        self.read_wait_queue
                            .push_back(GroupIndex::to_u32(group, idx));
                    } else {
                        self.read_wait_free_groups.push_back(group);
                    }
                }
                UserData::Send(idx) => {
                    //TODO maybe need to check flag with IORING_CQE_F_NOTIF
                    if !is_more {
                        self.send_free_queue.push_back(idx);
                    }
                }
                UserData::ProvideBuffers(_) => {}
            }
        }
    }
}

impl<const SEND_QUEUE: usize, const RECV_QUEUE: usize> UdpSocketGeneric
    for UdpSocket2IoUring<SEND_QUEUE, RECV_QUEUE>
{
    fn prepare(&mut self) {
        self.preapre_write();
        let groups = RECV_QUEUE / RECV_BUF_GROUP_SIZE as usize;
        for group in 0..groups {
            self.prepare_read(group as u16);
        }
    }

    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn add_send_to(&mut self, buf: &[u8], dest: SocketAddr) -> Result<usize, std::io::Error> {
        self.poll_completions();
        let pkt_idx = self.send_free_queue.pop_front().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "Send queue is full")
        })?;
        let pkt = &mut self.send_bufs[pkt_idx as usize];
        pkt.addr = SockAddr::from(dest);
        pkt.buf[..buf.len()].copy_from_slice(buf);
        pkt.iovecs[0].iov_len = buf.len();
        pkt.msg.msg_namelen = pkt.addr.len();

        let sendmsg_e = opcode::SendMsgZc::new(types::Fd(self.sockfd), &pkt.msg)
            .build()
            .user_data(UserData::Send(pkt_idx).into())
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
        if let Some(idx) = self.read_wait_queue.pop_front() {
            let pkt = &mut self.read_bufs[idx as usize];
            let msg: types::RecvMsgOut<'_> =
                types::RecvMsgOut::parse(pkt, &self.group_msghdr).expect("Should parse recv msg");
            let addr = unsafe {
                let storage = msg
                    .name_data()
                    .as_ptr()
                    .cast::<libc::sockaddr_storage>()
                    .read_unaligned();
                let len = msg.name_data().len().try_into().unwrap();
                socket2::SockAddr::new(storage, len)
            };
            let addr = addr.as_socket().expect("Should be addr");
            let payload = msg.payload_data().as_ptr();
            let payload_len = msg.payload_data().len();

            unsafe { Ok((std::slice::from_raw_parts(payload, payload_len), addr)) }
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "No data to read",
            ))
        }
    }

    fn finish_read_from(&mut self) -> Result<(), std::io::Error> {
        if self.read_wait_queue.is_empty() {
            if let Some(group) = self.read_wait_free_groups.pop_front() {
                self.prepare_read(group);
            }
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
        let mut socket1 = UdpSocket2IoUring::<16, 16>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<16, 16>::new("127.0.0.1:0");

        socket1.prepare();
        socket2.prepare();

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
        let mut socket1 = UdpSocket2IoUring::<16, 32>::new("127.0.0.1:0");
        let mut socket2 = UdpSocket2IoUring::<16, 32>::new("127.0.0.1:0");

        socket1.prepare();
        socket2.prepare();

        let addr1 = socket1.local_addr();
        let addr2 = socket2.local_addr();

        for _ in 0..3 {
            for i in 0..16 {
                socket1.add_send_to(&[i], addr2).expect("Should ok");
            }
            socket1.commit_send_to().expect("Should ok");
            std::thread::sleep(Duration::from_millis(100));

            for i in 0..16 {
                assert_eq!(socket2.recv_from().unwrap(), (vec![i].as_slice(), addr1));
            }
            socket2.finish_read_from().expect("Should ok");
        }
    }
}
