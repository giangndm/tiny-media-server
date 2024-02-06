use std::{collections::HashMap, net::SocketAddr};

#[derive(Debug)]
pub struct HttpRequest {
    pub req_id: u64,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct HttpResponse {
    pub req_id: u64,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub enum IoEvent<'a> {
    HttpRequest(HttpRequest),
    UdpSocketRecv {
        from: SocketAddr,
        to: SocketAddr,
        buf: &'a [u8],
    },
}

pub enum IoAction {
    HttpResponse(HttpResponse),
    UdpSocketSend {
        from: SocketAddr,
        to: SocketAddr,
        buf: Vec<u8>,
    },
}
