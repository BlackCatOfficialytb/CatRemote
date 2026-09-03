# The Plan (Optimized)

## ✅ Phase 0: Environment Validation & Capability Detection — **DONE**

*Implemented in `crates/catremote-env`*

* [x] **Compositor Capabilities:** Detect `wlr-screencopy-unstable-v1` and `ext-image-capture-source-v1`
* [x] **Portal Permission State:** Check `org.freedesktop.portal.ScreenCast` permission status
* [x] **GPU Encoder Detection:** Enumerate VA-API (`libva`) and NVENC (`nvml-wrapper`)
* [x] **PipeWire Version & Modules:** Verify ≥ 0.3.70; check required modules
* [x] **Kernel/Driver Checks:** DMA-BUF heaps (`/dev/dma_heap/*`), `libei` seat, `evdev` access
* [x] **CLI:** `catremote-env check --format json` outputs full capability report

---

## ✅ Phase 1: Core Capture & Encode Engine — **DONE**

*Implemented in `crates/catremote-capture`*

* [x] **Screen Capture Backends:**
    * [x] Primary: `ext-image-capture-source-v1` (headless, zero-copy)
    * [x] Fallback: `wlr-screencopy-unstable-v1`
* [x] **DMA-BUF Zero-Copy Pipeline:** Frames stay in VRAM; import directly to encoder
* [x] **Hardware Encoding Engine:**
    * [x] VA-API (Intel/AMD) via `libva` — primary
    * [x] NVENC (NVIDIA) stub — behind `nvenc` feature flag
    * [x] Codecs: H.264 Main/High, HEVC Main; configurable bitrate/preset/keyframe interval
* [x] **CLI Verification:** `catremote-capture record --output out.h264 --codec hevc --bitrate 20000`

---

## ✅ Phase 2: Audio & Input Engine — **DONE (with stubs)**

*Implemented in `crates/catremote-audio-input`*

* [x] **Audio Capture:** PipeWire loopback sink → Opus encoding (~20ms frames, 48kHz stereo)
* [x] **Opus Encoder:** `opus` crate (optional feature); passthrough fallback when disabled
* [x] **Input Injection via `libei`:** Stub implementation (`libei_stub` module); ready for real `libei` bindings
* [x] **Controller Mapping:** evdev → `libei` events (dead zones, sensitivity, button/axis mapping)
* [x] **Synchronization:** Common timestamp (µs since epoch) for audio/video/input correlation

> **Note:** Real `libei` bindings need `libei-sys`/`libei` crates when available on crates.io. Current stub compiles everywhere.

---

## 🎯 Phase 3: Network Transport Engine (Core) — **NEXT**

*Goal: Sub-10ms glass-to-glass latency over QUIC with adaptive bitrate.*

**Dependencies:** Phase 1 (encoded frames), Phase 2 (audio frames, input events)

* [ ] **QUIC Stack:** `quinn` crate; low-latency config (0-RTT, `max_idle_timeout=5s`, disable pacing)
* [ ] **Packetization:** NAL-aware fragmentation (H.264 FU-A / HEVC AP/FU) → MTU-sized datagrams; sequence numbers + frame boundaries + RTP-style headers
* [ ] **Congestion Control:** QUIC's built-in CUBIC + application-layer feedback (PLI/NACK via dedicated QUIC stream) → signal encoder bitrate changes
* [ ] **Forward Error Correction:** XOR parity packets for keyframe protection (optional, behind feature flag)
* [ ] **Multiplexing:** Single QUIC connection with 3 streams: video (unreliable), audio (unreliable), control (reliable)
* [ ] **Metrics Export:** Per-frame latency, bitrate, packet loss, RTT, jitter → exposed via IPC (Phase 4)

---

## 🤝 Phase 4: Local IPC & Control Plane

*Goal: Stable, versioned interface for external controllers (Python API, CLI, systemd).*

**Dependencies:** Phase 3 (needs to expose network stats/control)

* [ ] **Transport:** Unix Domain Socket (abstract namespace `@catremote`) + systemd socket activation (FD 3)
* [ ] **Protocol:** JSON-RPC 2.0 over newline-delimited frames; `v:1` version field; `request_id` for correlation
* [ ] **Core Methods:**
    * `start_stream { port, codec, bitrate, fps, keyframe_interval, secure: true, kem: "hybrid" }`
    * `stop_stream {}`
    * `set_bitrate { bitrate }`
    * `inject_input { device_id, events: [...] }`
    * `get_stats {} → { fps, latency_ms, bitrate, packets_lost, rtt_ms }`
    * `set_secure_mode { enabled: bool }`
    * `set_kem_algorithm { kem: "mlkem768" | "hybrid" }`
* [ ] **Events (server→client):** `stream_started`, `stream_stopped`, `stats`, `error`, `encoder_changed`, `secure_mode_changed`, `kem_algorithm_changed`
* [ ] **Secure Connection Toggle:** Default `secure: true`; if `false`, emit warning event + log conspicuous notice
* [ ] **KEM Algorithm Selection:**
    * `mlkem768` — Pure ML-KEM-768 (FIPS 203)
    * `hybrid` — **X25519MLKEM768** (default, recommended): X25519 + ML-KEM-768 hybrid per IETF draft

---

## 🔐 Phase 5: Security Hardening

*Goal: Defense-in-depth for network and local attack surfaces; post-quantum ready.*

**Dependencies:** Phase 3 (QUIC stack), Phase 4 (IPC)

* [ ] **Network Handshake (PQC):** QUIC with **ML-KEM-768** (FIPS 203) key encapsulation. Hybrid mode: ML-KEM + X25519. Implement via `quinn` + `pqcrypto-mlkem` / `oqs` bindings
* [ ] **Stream Encryption:** When `secure: true` (default), encrypt with **AES-256-GCM** (AES-NI). If `secure: false`, stream plaintext with conspicuous warning (log + IPC event)
* [ ] **Certificate/PSK Management:** PKI (certificate pinning) + PSK modes; PSK derived from pairing code (Phase 6)
* [ ] **IPC Security:** UDS credentials (`SO_PEERCRED`) — only same-UID processes; optional capability allowlist
* [ ] **Sandboxing:** Run core with minimal caps (`CAP_SYS_ADMIN` for DMA-BUF only via `capsh`/`systemd`); seccomp filter
* [ ] **Input Validation:** Strict JSON schema validation on all IPC messages; rate-limit input injection (max 1000 events/sec)
* [ ] **Key Rotation:** Periodic re-keying (configurable, default 1 hour) via ML-KEM re-encapsulation without stream interruption

---

## 🐍 Phase 6: Python API & Frontend Assembly

*Goal: Accessible Python wrapper and GUI for end users.*

**Dependencies:** Phase 4 (IPC), Phase 5 (security config)

* [ ] **FastAPI Signaling Server:** WebSocket handshake (WAN pairing codes), clipboard sync, session metadata; talks to core via IPC
* [ ] **PyQt6/PySide6 GUI:** System tray, config (bitrate, codec, encoder, KEM), connection list, stats overlay
* [ ] **Process Management:** Launch/monitor core via `asyncio.subprocess` with socket activation; auto-restart on crash; IPC health checks
* [ ] **Packaging:** `pyinstaller`/`cargo-bundle` for standalone distributable; systemd user unit template
* [ ] **Pairing Flow:** QR code / 6-digit code → derive PSK → configure Phase 5

---

## 📦 Crate Structure

```
catremote/
├── Cargo.toml (workspace)
├── crates/
│   ├── catremote-env/           # Phase 0 - capability detection
│   ├── catremote-capture/       # Phase 1 - video capture + encode
│   ├── catremote-audio-input/   # Phase 2 - audio capture + input injection
│   ├── catremote-transport/     # Phase 3 - QUIC transport (NEW)
│   ├── catremote-ipc/           # Phase 4 - JSON-RPC over UDS (NEW)
│   ├── catremote-crypto/        # Phase 5 - PQC, AES-GCM, key rotation (NEW)
│   └── catremote-python/        # Phase 6 - FastAPI + PyQt (NEW)
└── core/                        # Legacy (remove or merge)
```

---

## 🚀 Immediate Next Steps (Phase 3)

1. **Create `catremote-transport` crate** with `quinn` dependency
2. **Define packet format:** RTP-like header (sequence, timestamp, marker, payload type) + NAL fragments
3. **Implement QUIC connection manager** with 3 streams (video/audio/control)
4. **Add congestion feedback loop:** PLI/NACK → IPC → encoder bitrate adjustment
5. **Integration test:** `catremote-capture` → `catremote-transport` → loopback receive → decode verify

---

## 📋 Dependency Graph

```
Phase 0 (env) ──┬──→ Phase 1 (capture) ──┬──→ Phase 3 (transport) ──┬──→ Phase 4 (IPC) ──┬──→ Phase 5 (crypto) ──┬──→ Phase 6 (Python)
                │                         │                          │                     │
                └──→ Phase 2 (audio/input)┘                          └─────────────────────┘
```

**Critical Path:** 0 → 1 → 3 → 4 → 5 → 6  
**Parallel:** Phase 2 can develop alongside Phase 1; merges at Phase 3