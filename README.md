# Tiny webrtc media-server

This is a tiny media server for WebRTC SFU. It is designed for testing and optimizing WebRTC at high user scale, and will be applied to atm0s-media-server.

The project is kept simple and easy to understand, making it easy to test with different runtimes and optimization techniques like AF_XDP and Io-Uring.

## Architecture

Everything is SAN-I/O.

- Controller: init workers, bridge between shared I/O (http-server) and workers
- Worker: handle media packets, and send/recv to/from other workers
- Bus: crossbeam channel for inter-worker communication

## Features

- [x] Whip protocol
- [x] Whep protocol
- [x] Single port UDP
- [x] Io-Uring
- [ ] AF_XDP

### Updateds

## Benchmark results

### AWS c7i.xlarge 4cores 8GB

Default build release: `cargo build --release`

| Cores | Viewers | CPU % | Memory | Network(whep benchmark report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------------------ | ----------------- |
| 4     | 500     | 80%   | 4.55%  | 950 Mbps                       | 950 Mbps          |
| 4     | 1000    | 185%  | 9.0%   | 1.92 Gbps                      | 1.892 Gbps        |

Compared with Livekit:

| Cores | Viewers | CPU % | Memory | Network(cli report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------- | ----------------- |
| 4     | 500     | 100%  | 10%    | 522 Mbps            | 800Mbps           |
| 4     | 1000    | 199%  | 19.5%  | 819 Mbps            | 1.6 Gbps          |
