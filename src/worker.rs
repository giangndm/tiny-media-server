use faster_stun::attribute::*;
use faster_stun::*;
use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use str0m::{change::DtlsCert, media::KeyframeRequestKind};

use crossbeam::channel::{Receiver, Sender};

use crate::{
    io::{HttpResponse, IoAction, IoEvent},
    tasks::{ComposeTask, TrackMedia, WebrtcTask, WebrtcTaskInput, WebrtcTaskOutput},
};

pub enum BusEvent {
    TrackMedia(TrackMedia),
    TrackKeyframeRequest(u64, KeyframeRequestKind),
}

struct BusChannelContainer {
    sources: Vec<usize>,
    consumers: Vec<usize>,
}

struct TaskContainer {
    task: ComposeTask,
    remotes: Vec<SocketAddr>,
    sub_channels: Vec<u64>,
    pub_channels: Vec<u64>,
}

impl From<ComposeTask> for TaskContainer {
    fn from(task: ComposeTask) -> TaskContainer {
        TaskContainer {
            task,
            remotes: Vec::new(),
            sub_channels: Vec::new(),
            pub_channels: Vec::new(),
        }
    }
}

pub struct Worker {
    task_id_seed: usize,
    udp_addr: SocketAddr,
    udp_socket: UdpSocket,
    udp_buffer: [u8; 1500],
    ext_send: Sender<IoAction>,
    ext_recv: Receiver<IoEvent<'static>>,
    bus_send: Sender<BusEvent>,
    bus_recv: Receiver<BusEvent>,
    bus_channels: HashMap<u64, BusChannelContainer>,
    tasks: HashMap<usize, TaskContainer>,
    task_remotes: HashMap<SocketAddr, usize>,
    task_ufrags: HashMap<String, usize>,
    ended_tasks: Vec<usize>,
    dtls_cert: DtlsCert,
}

impl Worker {
    pub fn new(
        ext_send: Sender<IoAction>,
        ext_recv: Receiver<IoEvent<'static>>,
        bus_send: Sender<BusEvent>,
        bus_recv: Receiver<BusEvent>,
    ) -> Worker {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("Should bind to a udp port");

        Worker {
            task_id_seed: 0,
            udp_addr: socket.local_addr().expect("should have a local address"),
            udp_socket: socket,
            udp_buffer: [0; 1500],
            ext_send,
            ext_recv,
            bus_send,
            bus_recv,
            bus_channels: HashMap::new(),
            tasks: HashMap::new(),
            task_remotes: HashMap::new(),
            task_ufrags: HashMap::new(),
            ended_tasks: Vec::new(),
            dtls_cert: DtlsCert::new_openssl(),
        }
    }

    pub fn process_cycle(&mut self) -> Option<()> {
        let started = Instant::now();
        self.process_bus_recv();
        self.process_http();
        self.process_tick();
        self.pop_tasks(Instant::now());
        self.pop_ended_tasks();
        let elapsed = started.elapsed();
        let timeout = if elapsed < Duration::from_millis(1) {
            Duration::from_millis(1) - elapsed
        } else {
            Duration::from_micros(1)
        };
        self.process_udp(timeout);

        Some(())
    }

    fn process_tick(&mut self) {
        let instant = Instant::now();
        for (_task_id, task) in self.tasks.iter_mut() {
            task.task.tick(instant);
        }
    }

    fn process_http(&mut self) {
        while let Ok(event) = self.ext_recv.try_recv() {
            match event {
                IoEvent::HttpRequest(req) => match req.path.as_str() {
                    "/whip/endpoint" => {
                        let task_id = self.task_id_seed;
                        self.task_id_seed += 1;

                        let task = ComposeTask::Whip(crate::tasks::whip::WhipServerTask::new(
                            self.dtls_cert.clone(),
                            req,
                            vec![self.udp_addr],
                        ));
                        log::info!("Created whip task id: {}, ufrag: {}", task_id, task.ufrag());

                        self.task_ufrags.insert(task.ufrag(), task_id);
                        let mut task_container = task.into();
                        Self::pop_task(
                            Instant::now(),
                            task_id,
                            &mut task_container,
                            &self.udp_socket,
                            &self.ext_send,
                            &self.bus_send,
                            &mut self.bus_channels,
                            &mut self.ended_tasks,
                        );

                        self.tasks.insert(task_id, task_container);
                    }
                    "/whep/endpoint" => {
                        let task_id = self.task_id_seed;
                        self.task_id_seed += 1;

                        let task = ComposeTask::Whep(crate::tasks::whep::WhepServerTask::new(
                            self.dtls_cert.clone(),
                            req,
                            vec![self.udp_addr],
                        ));
                        log::info!("Created whep task id: {}, ufrag: {}", task_id, task.ufrag());

                        self.task_ufrags.insert(task.ufrag(), task_id);
                        let mut task_container = task.into();
                        Self::pop_task(
                            Instant::now(),
                            task_id,
                            &mut task_container,
                            &self.udp_socket,
                            &self.ext_send,
                            &self.bus_send,
                            &mut self.bus_channels,
                            &mut self.ended_tasks,
                        );

                        self.tasks.insert(task_id, task_container);
                    }
                    _ => {
                        self.ext_send
                            .send(IoAction::HttpResponse(HttpResponse {
                                req_id: req.req_id,
                                status: 404,
                                headers: Default::default(),
                                body: b"Not Found".to_vec(),
                            }))
                            .unwrap();
                    }
                },
                _ => panic!("Should not receive this event."),
            }
        }
    }

    fn process_bus_recv(&mut self) {
        while let Ok(event) = self.bus_recv.try_recv() {
            match event {
                BusEvent::TrackMedia(media) => {
                    if let Some(channel) = self.bus_channels.get(&media.track_id) {
                        for consumer in &channel.consumers {
                            if let Some(task) = self.tasks.get_mut(consumer) {
                                task.task.input(
                                    Instant::now(),
                                    WebrtcTaskInput::TrackMedia(media.clone()),
                                );
                            }
                        }
                    }
                }
                BusEvent::TrackKeyframeRequest(track_id, kind) => {
                    if let Some(channel) = self.bus_channels.get(&track_id) {
                        for source in &channel.sources {
                            if let Some(task) = self.tasks.get_mut(source) {
                                task.task.input(
                                    Instant::now(),
                                    WebrtcTaskInput::RequestKeyframeTrack { track_id, kind },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn process_udp(&mut self, timeout: Duration) {
        log::trace!("Processing udp with timeout: {:?}", timeout);
        self.udp_socket
            .set_read_timeout(Some(timeout))
            .expect("Should set a timeout");
        while let Ok((size, remote)) = self.udp_socket.recv_from(&mut self.udp_buffer) {
            let now = Instant::now();
            log::trace!("Received udp packet from {:?}, size: {}", remote, size);
            let slot = if let Some(task_id) = self.task_remotes.get(&remote) {
                if let Some(task) = self.tasks.get_mut(task_id) {
                    Some((*task_id, task))
                } else {
                    None
                }
            } else {
                if let Some(stun_username) = Self::get_stun_username(&self.udp_buffer[..size]) {
                    log::warn!(
                        "Received a stun packet from an unknown remote: {:?}, username {}",
                        remote,
                        stun_username
                    );
                    if let Some(task_id) = self.task_ufrags.get(stun_username).cloned() {
                        log::info!("Mapping remote {:?} to task {}", remote, task_id);
                        self.task_remotes.insert(remote, task_id);
                        if let Some(task) = self.tasks.get_mut(&task_id) {
                            task.remotes.push(remote);
                            Some((task_id, task))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((task_id, task)) = slot {
                task.task.input(
                    now,
                    IoEvent::UdpSocketRecv {
                        from: remote,
                        to: self.udp_addr,
                        buf: &self.udp_buffer[..size],
                    }
                    .into(),
                );

                //we should pop_task here because str0m don't store pending incomming packets in queue, only flag. If call here we lost some events
                Self::pop_task(
                    now,
                    task_id,
                    task,
                    &self.udp_socket,
                    &self.ext_send,
                    &self.bus_send,
                    &mut self.bus_channels,
                    &mut self.ended_tasks,
                )
            }
        }
    }

    fn get_stun_username(buf: &[u8]) -> Option<&str> {
        let mut attributes = Vec::new();
        let message = MessageReader::decode(buf, &mut attributes).unwrap();
        message
            .get::<UserName>()
            .map(|u| u.split(':').next().expect("Should have a pair username"))
    }

    fn pop_tasks(&mut self, now: Instant) {
        for (task_id, task) in self.tasks.iter_mut() {
            Self::pop_task(
                now,
                *task_id,
                task,
                &self.udp_socket,
                &self.ext_send,
                &self.bus_send,
                &mut self.bus_channels,
                &mut self.ended_tasks,
            );
        }
    }

    fn pop_task(
        now: Instant,
        task_id: usize,
        task: &mut TaskContainer,
        udp_socket: &UdpSocket,
        ext_send: &Sender<IoAction>,
        bus_send: &Sender<BusEvent>,
        bus_channels: &mut HashMap<u64, BusChannelContainer>,
        ended_tasks: &mut Vec<usize>,
    ) {
        while let Some(action) = task.task.pop_action(now) {
            match action {
                WebrtcTaskOutput::Io(IoAction::UdpSocketSend { from: _, to, buf }) => {
                    if let Err(e) = udp_socket.send_to(&buf, to) {
                        log::error!("Failed to send udp packet to {to}: {e}");
                    }
                }
                WebrtcTaskOutput::Io(IoAction::HttpResponse(res)) => {
                    if let Err(e) = ext_send.try_send(IoAction::HttpResponse(res)) {
                        log::error!("Failed to send response to controller: {e}");
                    }
                }
                WebrtcTaskOutput::TrackMedia(media) => {
                    if let Err(e) = bus_send.try_send(BusEvent::TrackMedia(media)) {
                        log::error!("Failed to send track media to bus: {}", e.to_string());
                    }
                }
                WebrtcTaskOutput::RequestKeyframeTrack { track_id, kind } => {
                    if let Err(e) =
                        bus_send.try_send(BusEvent::TrackKeyframeRequest(track_id, kind))
                    {
                        log::error!("Failed to send keyframe request to bus: {}", e.to_string());
                    }
                }
                WebrtcTaskOutput::TaskEnded => {
                    log::info!("Task {task_id} ended");
                    ended_tasks.push(task_id);
                }
                WebrtcTaskOutput::PublishTrack { track_id } => {
                    log::info!("Task {task_id} published track {track_id}");
                    bus_channels
                        .entry(track_id)
                        .or_insert(BusChannelContainer {
                            sources: Vec::new(),
                            consumers: Vec::new(),
                        })
                        .sources
                        .push(task_id);
                    task.pub_channels.push(track_id);
                }
                WebrtcTaskOutput::SubscribeTrack { track_id } => {
                    log::info!("Task {task_id} subscribed to track {track_id}");
                    bus_channels
                        .entry(track_id)
                        .or_insert(BusChannelContainer {
                            sources: Vec::new(),
                            consumers: Vec::new(),
                        })
                        .consumers
                        .push(task_id);
                    task.sub_channels.push(track_id);
                }
            }
        }
    }

    fn pop_ended_tasks(&mut self) {
        for task_id in self.ended_tasks.drain(..) {
            let container = self.tasks.remove(&task_id).expect("Should have a task");
            for remote in container.remotes {
                self.task_remotes.remove(&remote);
            }
            self.task_ufrags.remove(&container.task.ufrag());
            for track_id in container.sub_channels {
                if let Some(channel) = self.bus_channels.get_mut(&track_id) {
                    channel.consumers.retain(|c| *c != task_id);
                    if channel.consumers.is_empty() && channel.sources.is_empty() {
                        self.bus_channels.remove(&track_id);
                    }
                }
            }
            for track_id in container.pub_channels {
                if let Some(channel) = self.bus_channels.get_mut(&track_id) {
                    channel.consumers.retain(|c| *c != task_id);
                    if channel.consumers.is_empty() && channel.sources.is_empty() {
                        self.bus_channels.remove(&track_id);
                    }
                }
            }
        }
    }
}
