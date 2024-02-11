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

### Updateds

## Benchmark results

- `sendmmsg` not improving much.
- upgrade from c6a.xlarge to c7i.xlarge, 4cores 8GB, 1.5x performance improvement, now 1core can serve ~500 viewers.

### AWS c7i.xlarge 4cores 8GB

Default build release: `cargo build --release`

| Cores | Viewers | CPU % | Memory | Network(whep benchmark report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------------------ | ----------------- |
| 4     | 500     | 104%  | 7.1%   | 950 Mbps                       | 950 Mbps          |
| 4     | 1000    | 210%  | 14.2%  | 1.92 Gbps                      | 1.92 Gbps         |

Compared with Livekit:

| Cores | Viewers | CPU % | Memory | Network(cli report) | Network OS report |
| ----- | ------- | ----- | ------ | ------------------- | ----------------- |
| 4     | 500     | 100%  | 10%   | 522 Mbps                       | 800Mbps              |
| 4     | 1000    | 190%  | 18.6%  | 891 Mbps                      | 1.6 Gbps              |

### AWS c6a.xlarge 4cores 8GB

Default build release: `cargo build --release`

| Cores | Viewers | CPU % | Memory | Network(whep benchmark report) | Network OS report |
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
