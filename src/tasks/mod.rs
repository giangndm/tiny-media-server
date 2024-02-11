use std::{
    hash::{Hash, Hasher},
    time::Instant,
};

use bytes::Bytes;
use str0m::{
    media::{KeyframeRequestKind, MediaKind, MediaTime},
    rtp::{RtpHeader, RtpPacket, SeqNo},
};

use crate::io::{IoAction, IoEvent};

pub mod whep;
pub mod whip;

#[derive(Debug, Clone)]
pub struct TrackMedia {
    pub track_id: u64,
    /// Extended sequence number to avoid having to deal with ROC.
    pub seq_no: SeqNo,

    /// Extended RTP time in the clock frequency of the codec. To avoid dealing with ROC.
    ///
    /// For a newly scheduled outgoing packet, the clock_rate is not correctly set until
    /// we do the poll_output().
    pub time: MediaTime,

    /// Parsed RTP header.
    pub header: RtpHeader,

    /// RTP payload. This contains no header.
    pub payload: Bytes,

    /// str0m server timestamp.
    ///
    /// This timestamp has nothing to do with RTP itself. For outgoing packets, this is when
    /// the packet was first handed over to str0m and enqueued in the outgoing send buffers.
    /// For incoming packets it's the time we received the network packet.
    pub timestamp: Instant,
}

impl TrackMedia {
    pub fn from_raw(track_id: u64, rtp: RtpPacket) -> Self {
        let header = rtp.header;
        let payload = rtp.payload;
        let time = rtp.time;
        let timestamp = rtp.timestamp;
        let seq_no = rtp.seq_no;

        Self {
            track_id,
            seq_no,
            time,
            header,
            payload,
            timestamp,
        }
    }
}

pub enum WebrtcTaskInput<'a> {
    Io(IoEvent<'a>),
    TrackMedia(TrackMedia),
    RequestKeyframeTrack {
        track_id: u64,
        kind: KeyframeRequestKind,
    },
}

impl<'a> From<IoEvent<'a>> for WebrtcTaskInput<'a> {
    fn from(event: IoEvent) -> WebrtcTaskInput {
        WebrtcTaskInput::Io(event)
    }
}

pub enum WebrtcTaskOutput {
    Io(IoAction),
    TrackMedia(TrackMedia),
    TaskEnded,
    PublishTrack {
        track_id: u64,
    },
    SubscribeTrack {
        track_id: u64,
    },
    RequestKeyframeTrack {
        track_id: u64,
        kind: KeyframeRequestKind,
    },
}

impl From<IoAction> for WebrtcTaskOutput {
    fn from(action: IoAction) -> WebrtcTaskOutput {
        WebrtcTaskOutput::Io(action)
    }
}

pub trait WebrtcTask {
    fn ufrag(&self) -> String;
    /// return true if have action to process
    fn tick(&mut self, now: Instant) -> bool;
    /// return true if have action to process
    fn input<'b>(&mut self, now: Instant, event: WebrtcTaskInput<'b>) -> bool;
    fn pop_action(&mut self, now: Instant) -> Option<WebrtcTaskOutput>;
}

pub enum ComposeTask {
    Whip(whip::WhipServerTask),
    Whep(whep::WhepServerTask),
}

impl WebrtcTask for ComposeTask {
    fn ufrag(&self) -> String {
        match self {
            ComposeTask::Whip(task) => task.ufrag(),
            ComposeTask::Whep(task) => task.ufrag(),
        }
    }

    fn tick(&mut self, instant: Instant) -> bool {
        match self {
            ComposeTask::Whip(task) => task.tick(instant),
            ComposeTask::Whep(task) => task.tick(instant),
        }
    }

    fn input<'b>(&mut self, now: Instant, event: WebrtcTaskInput<'b>) -> bool {
        match self {
            ComposeTask::Whip(task) => task.input(now, event),
            ComposeTask::Whep(task) => task.input(now, event),
        }
    }

    fn pop_action(&mut self, now: Instant) -> Option<WebrtcTaskOutput> {
        match self {
            ComposeTask::Whip(task) => task.pop_action(now),
            ComposeTask::Whep(task) => task.pop_action(now),
        }
    }
}

pub fn track_id_builder(channel: &str, kind: MediaKind) -> u64 {
    //generate form hash of "channel/kind"
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    channel.hash(&mut hasher);
    kind.hash(&mut hasher);
    hasher.finish()
}
