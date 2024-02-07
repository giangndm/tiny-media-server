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
- [ ] Io-Uring
- [ ] AF_XDP

## Benchmarks

Based: aws c6a.large 4cores 8GB

Update1: `sendmmsg` not improving much.

Default build release: `cargo build --release`

| Cores | Viewers | CPU % | Memory | Network(whep benchamrk report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------------------ | ----------------- |
| 4     | 100     | 24%   | 1.8%   | 160 Mbps                       | TODO              |
| 4     | 200     | 66%   | 3.0%   | 388 Mbps                       | TODO              |
| 4     | 500     | 169%  | 7.6%   | 960 Mbps                       | TODO              |
| 4     | 1000    | 355%  | 15.2%  | 1.96 Gbps                      | TODO              |
| 4     | 2000    | TODO  | TODO   | TODO                           | TODO              |

Compared with Livekit:

| Cores | Viewers | CPU % | Memory | Network(cli report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------- | ----------------- |
|       | 100     | 34%   | 2.2%   | 90 Mbps             | TODO              |
|       | 200     | 58%   | 4.3%   | 197 mbps            | TODO              |
|       | 500     | 136%  | 9.9%   | 465 mbps            | TODO              |
|       | 1000    | 292%  | 19.5%  | 823 mbps            | TODO              |
|       | 2000    | TODO  | TODO   | TODO                | TODO              |
