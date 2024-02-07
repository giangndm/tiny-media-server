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

Based: aws c6a.large

| Cores | Viewers | CPU % | Memory |
|-------|---------|-------|--------|
| 1     | 100     | TODO  | TODO   |
| 2     | 200     | TODO  | TODO   |
| 3     | 400     | TODO  | TODO   |
| 3     | 1000     | TODO  | TODO   |
| 3     | 2000     | TODO  | TODO   |