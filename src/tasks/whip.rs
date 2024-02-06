use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    time::Instant,
};

use str0m::{
    change::{DtlsCert, SdpOffer},
    media::MediaKind,
    net::{Protocol, Receive},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};

use crate::{
    io::{HttpRequest, HttpResponse, IoAction, IoEvent},
    tasks::{track_id_builder, TrackMedia},
};

use super::{WebrtcTask, WebrtcTaskInput, WebrtcTaskOutput};

pub struct WhipServerTask {
    ice_ufrag: String,
    timeout: Option<Instant>,
    rtc: Rtc,
    outputs: VecDeque<WebrtcTaskOutput>,
    audio_track_id: u64,
    video_track_id: u64,
}

impl WhipServerTask {
    pub fn new(
        dtls_cert: DtlsCert,
        req: HttpRequest,
        local_addrs: Vec<SocketAddr>,
    ) -> WhipServerTask {
        log::debug!(
            "WhipServerTask::new req: {} addr {:?}",
            req.path,
            local_addrs
        );
        let rtc_config = Rtc::builder()
            .set_rtp_mode(true)
            .set_ice_lite(true)
            .set_dtls_cert(dtls_cert);

        let channel = req
            .headers
            .get("Authorization")
            .map(|v| v.clone())
            .unwrap_or_else(|| "demo".to_string());
        let ice_ufrag = rtc_config.local_ice_credentials().ufrag.clone();

        let mut rtc = rtc_config.build();
        rtc.direct_api().enable_twcc_feedback();

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

        WhipServerTask {
            ice_ufrag,
            timeout: None,
            rtc,
            outputs: VecDeque::from(vec![IoAction::HttpResponse(HttpResponse {
                req_id: req.req_id,
                status: 200,
                headers: HashMap::from([
                    ("Content-Type".to_string(), "application/sdp".to_string()),
                    ("Location".to_string(), "/whip/endpoint/1234".to_string()),
                ]),
                body: answer.to_sdp_string().as_bytes().to_vec(),
            })
            .into()]),
            audio_track_id: track_id_builder(&channel, MediaKind::Audio),
            video_track_id: track_id_builder(&channel, MediaKind::Video),
        }
    }
}

impl WebrtcTask for WhipServerTask {
    fn ufrag(&self) -> String {
        self.ice_ufrag.clone()
    }

    fn tick(&mut self, now: Instant) -> bool {
        if let Some(timeout) = self.timeout {
            if now >= timeout {
                if let Err(e) = self.rtc.handle_input(Input::Timeout(now)) {
                    log::error!("Error handling timeout: {}", e);
                }
                log::debug!("clear timeout after handled");
                self.timeout = None;
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
                log::debug!("clear timeout with udp");
                self.timeout = None;
                true
            }
            _ => panic!("Should not receive this event."),
        }
    }

    fn pop_action(&mut self, now: Instant) -> Option<WebrtcTaskOutput> {
        if let Some(o) = self.outputs.pop_front() {
            return Some(o);
        }

        if let Some(timeout) = self.timeout {
            if timeout > now {
                return None;
            }
        }

        match self.rtc.poll_output().ok()? {
            Output::Timeout(timeout) => {
                self.timeout = Some(timeout);
                log::debug!("set timeout after {:?}", timeout - now);
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
                    log::info!("WhipServerTask connected");
                    None
                }
                Event::MediaAdded(media) => {
                    log::info!("WhipServerTask media added: {:?}", media);
                    None
                }
                Event::IceConnectionStateChange(state) => match state {
                    IceConnectionState::Disconnected => Some(WebrtcTaskOutput::TaskEnded),
                    _ => None,
                },
                Event::RtpPacket(rtp) => {
                    let track_id = if *rtp.header.payload_type == 111 {
                        self.audio_track_id
                    } else {
                        self.video_track_id
                    };
                    Some(WebrtcTaskOutput::TrackMedia(TrackMedia::from_raw(
                        track_id, rtp,
                    )))
                }
                _ => None,
            },
        }
    }
}
