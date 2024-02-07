use std::{net::IpAddr, sync::Arc, thread::JoinHandle};

use bus::Bus;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::Mutex;

use crate::{
    io::{IoAction, IoEvent},
    worker::Worker,
};

struct WorkerSlot {
    join: JoinHandle<()>,
    sender: Sender<IoEvent<'static>>,
}

pub struct Controller {
    count: usize,
    joins: Vec<WorkerSlot>,
    worker_recv: Receiver<IoAction>,
}

impl Controller {
    pub fn new(workers: usize, ip_addr: IpAddr) -> Controller {
        let bus = Arc::new(Mutex::new(Bus::new(1000)));
        let (worker_send, worker_recv) = crossbeam::channel::bounded(100);
        let mut joins = Vec::new();
        for _ in 0..workers {
            let (sender, receiver) = crossbeam::channel::bounded(100);
            let mut worker = Worker::new(
                ip_addr,
                worker_send.clone(),
                receiver,
                bus.clone(),
                bus.lock().add_rx(),
            );
            let thread = std::thread::spawn(move || {
                while let Some(_) = worker.process_cycle() {
                    // Do nothing
                }
            });
            joins.push(WorkerSlot {
                join: thread,
                sender,
            });
        }

        Controller {
            count: 0,
            joins,
            worker_recv,
        }
    }

    pub fn input<'a>(&mut self, event: IoEvent<'a>) {
        match event {
            IoEvent::HttpRequest(req) => {
                let slot_index = self.count % self.joins.len();
                let slot = &mut self.joins[slot_index];
                if let Err(e) = slot.sender.try_send(IoEvent::HttpRequest(req)) {
                    log::error!("Failed to send request to worker {slot_index}: {e}");
                }
                self.count += 1;
            }
            _ => panic!("Should not receive this event."),
        }
    }

    pub fn pop_action(&mut self) -> Option<IoAction> {
        self.worker_recv.try_recv().ok()
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        for join in self.joins.drain(..) {
            join.join
                .join()
                .expect("Should wait child thread to finish.");
        }
    }
}
