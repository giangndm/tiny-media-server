use clap::Parser;
use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::Path;
use std::{collections::HashMap, time::Duration};

use tiny_http::{Header, Method, Response, Server};
use tiny_media_server::io::IoAction;
use tiny_media_server::{
    controller::Controller,
    io::{HttpRequest, IoEvent},
};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Media Server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Http port
    #[arg(env, long, default_value = "0.0.0.0:8000")]
    http_addr: SocketAddr,

    /// Number of workers
    #[arg(env, long, default_value_t = 4)]
    workers: usize,

    /// Listen address for media data
    #[arg(env, long, default_value = "127.0.0.1")]
    listen_addr: IpAddr,
}

fn main() {
    let udp_server = UdpSocket::bind("127.0.0.1:40000").expect("");
    let mut buf = [0; 1500];
    while let Ok((size, addr)) = udp_server.recv_from(&mut buf) {
        println!("Received {} bytes from {}", size, addr);
    }

    let args: Args = Args::parse();
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let mut req_id = 0;
    let mut reqs = HashMap::new();
    let server = Server::http(args.http_addr).unwrap();
    log::info!("server started at port {}", args.http_addr);
    let mut controller = Controller::new(args.workers, args.listen_addr);

    loop {
        if let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(100)) {
            if request.url().starts_with("/public") {
                let file = File::open(&Path::new(&format!(".{}", request.url())))
                    .expect("Should open file.");
                let mut response = tiny_http::Response::from_file(file);
                if request.url().ends_with(".js") {
                    response.add_header(
                        Header::from_bytes("Content-Type", "application/javascript").unwrap(),
                    );
                } else if request.url().ends_with(".css") {
                    response.add_header(Header::from_bytes("Content-Type", "text/css").unwrap());
                }
                request.respond(response).expect("Should respond file.");
                continue;
            }

            if request.method().eq(&Method::Options) {
                let mut response = Response::from_string("OK");
                //setting CORS
                response
                    .add_header(Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap());
                response.add_header(
                    Header::from_bytes(
                        "Access-Control-Allow-Methods",
                        "GET, POST, PATCH, DELETE, OPTIONS",
                    )
                    .unwrap(),
                );
                response
                    .add_header(Header::from_bytes("Access-Control-Allow-Headers", "*").unwrap());
                response.add_header(
                    Header::from_bytes("Access-Control-Allow-Credentials", "true").unwrap(),
                );

                request.respond(response).expect("Should respond options.");
                continue;
            }

            log::info!(
                "received request_id {} method: {}, url: {}",
                req_id,
                request.method(),
                request.url(),
            );

            let io_event = IoEvent::HttpRequest(HttpRequest {
                req_id,
                method: request.method().to_string(),
                path: request.url().to_string(),
                headers: request
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string(), h.value.to_string()))
                    .collect(),
                body: request.as_reader().bytes().map(|b| b.unwrap()).collect(),
            });
            controller.input(io_event);
            reqs.insert(req_id, request);
            req_id += 1;
        }

        while let Some(action) = controller.pop_action() {
            match action {
                IoAction::HttpResponse(res) => {
                    log::info!(
                        "sending response for request_id {}, status {}",
                        res.req_id,
                        res.status
                    );
                    let req = reqs.remove(&res.req_id).expect("Should have a request.");
                    let mut response = Response::from_data(res.body).with_status_code(res.status);
                    for (k, v) in res.headers {
                        response
                            .add_header(Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
                    }
                    response.add_header(
                        Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                    );
                    response.add_header(
                        Header::from_bytes(
                            "Access-Control-Allow-Methods",
                            "GET, POST, PATCH, DELETE, OPTIONS",
                        )
                        .unwrap(),
                    );
                    response.add_header(
                        Header::from_bytes("Access-Control-Allow-Headers", "*").unwrap(),
                    );
                    response.add_header(
                        Header::from_bytes("Access-Control-Allow-Credentials", "true").unwrap(),
                    );
                    req.respond(response).unwrap();
                }
                _ => panic!("Should not receive this event."),
            }
        }
    }
}
