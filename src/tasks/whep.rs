use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    time::Instant,
};

use str0m::{
    change::SdpOffer,
    media::{MediaKind, Mid},
    net::{Protocol, Receive},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};

use crate::{
    io::{HttpRequest, HttpResponse, IoAction, IoEvent},
    tasks::track_id_builder,
};

use super::{WebrtcTask, WebrtcTaskInput, WebrtcTaskOutput};

pub struct WhepServerTask {
    channel: String,
    ice_ufrag: String,
    timeout: Option<Instant>,
    rtc: Rtc,
    outputs: VecDeque<WebrtcTaskOutput>,
    audio_mid: Option<Mid>,
    video_mid: Option<Mid>,
}

impl WhepServerTask {
    pub fn new(req: HttpRequest, local_addrs: Vec<SocketAddr>) -> WhepServerTask {
        log::debug!(
            "WhepServerTask::new req: {} addr {:?}",
            req.path,
            local_addrs
        );
        let rtc_config = Rtc::builder().set_rtp_mode(true).set_ice_lite(true);

        let channel = req
            .headers
            .get("Authorization")
            .map(|v| v.clone())
            .unwrap_or_else(|| "demo".to_string());
        let ice_ufrag = rtc_config.local_ice_credentials().ufrag.clone();

        let mut rtc = rtc_config.build();

        for addr in local_addrs {
            rtc.add_local_candidate(
                Candidate::host(addr, Protocol::Udp).expect("Should create candidate"),
            );
        }

        let offer = SdpOffer::from_sdp_string(&String::from_utf8_lossy(&req.body))
            .expect("Should parse offer");
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .expect("Should accept offer");

        WhepServerTask {
            channel,
            ice_ufrag,
            timeout: None,
            rtc,
            outputs: VecDeque::from(vec![IoAction::HttpResponse(HttpResponse {
                req_id: req.req_id,
                status: 200,
                headers: HashMap::from([
                    ("Content-Type".to_string(), "application/sdp".to_string()),
                    ("Location".to_string(), "/whep/endpoint/1234".to_string()),
                ]),
                body: answer.to_sdp_string().as_bytes().to_vec(),
            })
            .into()]),
            audio_mid: None,
            video_mid: None,
        }
    }
}

impl WebrtcTask for WhepServerTask {
    fn ufrag(&self) -> String {
        self.ice_ufrag.clone()
    }

    fn tick(&mut self, now: Instant) -> bool {
        if let Some(timeout) = self.timeout {
            if now >= timeout {
                if let Err(e) = self.rtc.handle_input(Input::Timeout(now)) {
                    log::error!("Error handling timeout: {}", e);
                }
                return true;
            }
        }
        false
    }

    fn input<'b>(&mut self, now: Instant, event: WebrtcTaskInput<'b>) -> bool {
        match event {
            WebrtcTaskInput::Io(IoEvent::HttpRequest(_req)) => {
                todo!()
            }
            WebrtcTaskInput::Io(IoEvent::UdpSocketRecv { from, to, buf }) => {
                if let Err(e) = self.rtc.handle_input(Input::Receive(
                    now,
                    Receive::new(Protocol::Udp, from, to, buf).expect("Should parse udp"),
                )) {
                    log::error!("Error handling udp: {}", e);
                }
                true
            }
            WebrtcTaskInput::TrackMedia(media) => {
                let (mid, nackable) = if *media.header.payload_type == 111 {
                    //audio
                    (self.audio_mid, false)
                } else {
                    (self.video_mid, true)
                };

                if let Some(mid) = mid {
                    if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
                        if let Err(e) = stream.write_rtp(
                            media.header.payload_type,
                            media.seq_no,
                            media.header.timestamp,
                            media.timestamp,
                            media.header.marker,
                            media.header.ext_vals,
                            nackable,
                            media.payload,
                        ) {
                            log::error!("Error writing rtp: {}", e);
                        }
                    }
                }

                true
            }
        }
    }

    fn pop_action(&mut self) -> Option<WebrtcTaskOutput> {
        if let Some(o) = self.outputs.pop_front() {
            return Some(o);
        }

        match self.rtc.poll_output().ok()? {
            Output::Timeout(timeout) => {
                self.timeout = Some(timeout);
                None
            }
            Output::Transmit(send) => Some(
                IoAction::UdpSocketSend {
                    from: send.source,
                    to: send.destination,
                    buf: send.contents.into(),
                }
                .into(),
            ),
            Output::Event(e) => match e {
                Event::Connected => {
                    log::info!("WhepServerTask connected");
                    self.outputs.push_back(WebrtcTaskOutput::SubscribeTrack {
                        track_id: track_id_builder(&self.channel, MediaKind::Audio),
                    });
                    self.outputs.push_back(WebrtcTaskOutput::SubscribeTrack {
                        track_id: track_id_builder(&self.channel, MediaKind::Video),
                    });
                    None
                }
                Event::MediaAdded(media) => {
                    log::info!("WhepServerTask media added: {:?}", media);
                    if media.kind == MediaKind::Audio {
                        self.audio_mid = Some(media.mid);
                    } else {
                        self.video_mid = Some(media.mid);
                    }
                    None
                }
                Event::IceConnectionStateChange(state) => match state {
                    IceConnectionState::Disconnected => Some(WebrtcTaskOutput::TaskEnded),
                    _ => None,
                },
                _ => None,
            },
        }
    }
}
