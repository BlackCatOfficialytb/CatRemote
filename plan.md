# The Plan

## 🛠️ Phase 0: Environment Validation & Capability Detection

*Goal: Verify the runtime environment supports all required primitives before building.*

* [ ] **Compositor Capabilities:** Detect Wayland compositor support for `wlr-screencopy-unstable-v1` and/or `ext-image-capture-source-v1` (preferred over portal for headless).
* [ ] **Portal Permission State:** Check existing `org.freedesktop.portal.ScreenCast` permissions; document persistent grant flow for daemon use.
* [ ] **GPU Encoder Detection:** Enumerate VA-API (`vainfo`) and NVENC (`nvidia-smi`, `nvencodeapi`) capabilities; select primary/fallback encoder.
* [ ] **PipeWire Version & Modules:** Verify PipeWire ≥ 0.3.70; confirm `libpipewire-module-virtual-sink` and `libpipewire-module-loopback` available.
* [ ] **Kernel/Driver Checks:** DMA-BUF support (`/dev/dma_heap/*`), `libei` seat support, `evdev` access for controller mapping.

---

## 🛠️ Phase 1: Core Capture & Encode Engine (Rust)

*Goal: Headless Rust daemon that captures Wayland output via zero-copy DMA-BUF and hardware-encodes entirely in VRAM.*

* [ ] **Screen Capture Backend:**
    * [ ] Primary: `wlr-screencopy-unstable-v1` + `ext-image-capture-source-v1` (no user prompt, works headless)
    * [ ] Fallback: `org.freedesktop.portal.ScreenCast` via `pipewire-rs` (requires persistent portal permission)
* [ ] **DMA-BUF Zero-Copy Pipeline:** Frames stay in VRAM as DMA-BUFs; import directly into encoder without CPU mapping.
* [ ] **Hardware Encoding Engine:**
    * [ ] VA-API path (Intel/AMD) via `libva`/`vaapi-rs` — primary target
    * [ ] NVENC path (NVIDIA) via `nvencodeapi` bindings — optional, behind feature flag
    * [ ] Codec support: H.264 baseline/high, HEVC main; configurable bitrate/preset/keyframe interval
* [ ] **CLI Verification:** `core --record output.h264 --codec hevc --bitrate 20M` produces valid, hardware-accelerated stream.

---

## 🔊 Phase 2: Audio & Input Engine (Core)

*Goal: Capture system audio and inject input events — all inside the Rust core.*

* [ ] **Audio Capture:** PipeWire virtual loopback sink → Opus encoding (low-latency, ~20ms frames).
* [ ] **Input Injection via `libei`:** Bind `libei` in Rust; implement EI context → device → pointer/keyboard/touch events.
* [ ] **Controller Mapping:** evdev/gamepad → `libei` event translation (dead zones, button remap, axis scaling).
* [ ] **Synchronization:** Timestamp audio/video/input streams to common clock (CLOCK_MONOTONIC) for client-side lip-sync.

---

## 🌐 Phase 3: Network Transport Engine (Core)

*Goal: Sub-10ms glass-to-glass latency over QUIC with adaptive bitrate.*

* [ ] **QUIC Stack:** `quinn` crate; configure for low-latency (0-RTT, small `max_idle_timeout`, disable pacing for real-time).
* [ ] **Packetization:** NAL-aware fragmentation (H.264 FU-A / HEVC AP/FU) into MTU-sized datagrams; sequence numbers + frame boundaries.
* [ ] **Congestion Control:** Start with QUIC's built-in CUBIC; add application-layer feedback (PLI/NACK via RTCP-style messages over QUIC stream) to signal encoder bitrate changes.
* [ ] **Forward Error Correction (optional):** XOR parity packets for keyframe protection.
* [ ] **Metrics Export:** Per-frame latency, bitrate, packet loss, RTT — exposed via IPC for monitoring.

---

## 🤝 Phase 4: Local IPC & Control Plane

*Goal: Stable, versioned interface for external controllers (Python API, CLI, systemd).*

* [ ] **Transport:** Unix Domain Socket (abstract namespace) + systemd socket activation (FD 3).
* [ ] **Protocol:** JSON-RPC 2.0 over newline-delimited frames; `v:1` version field; `request_id` for correlation.
* [ ] **Core Methods:**
    * `start_stream { port, codec, bitrate, fps, keyframe_interval, secure: true, kem: "hybrid" }`
    * `stop_stream {}`
    * `set_bitrate { bitrate }`
    * `inject_input { device_id, events: [...] }`
    * `get_stats {} → { fps, latency_ms, bitrate, packets_lost }`
    * `set_secure_mode { enabled: bool }` — toggle encryption at runtime
    * `set_kem_algorithm { kem: "mlkem768" | "ecdh" | "hybrid" }` — configure handshake KEM
* [ ] **Events (server→client):** `stream_started`, `stream_stopped`, `stats`, `error`, `encoder_changed`, `secure_mode_changed`, `kem_algorithm_changed`.
* [ ] **Secure Connection Toggle:** Default `secure: true` in `start_stream`; if `false`, emit warning event and log conspicuous notice.
* [ ] **KEM Algorithm Selection:** `kem` parameter in `start_stream` and `set_kem_algorithm`:
    * `mlkem768` — Pure ML-KEM-768 (FIPS 203)
    * `hybrid` — **X25519MLKEM768** (default, recommended): X25519 + ML-KEM-768 hybrid per IETF draft

---

## 🔐 Phase 5: Security Hardening

*Goal: Defense-in-depth for network and local attack surfaces; post-quantum ready.*

* [ ] **Network Handshake (PQC):** QUIC with **ML-KEM-768** (FIPS 203) key encapsulation for post-quantum forward secrecy. Hybrid mode: ML-KEM + X25519 during transition. Implemented via `quinn` + `pqcrypto`/`oqs` bindings.
* [ ] **Stream Encryption:** When `secure: true` (default), encrypt QUIC payloads with **AES-256-GCM** (hardware-accelerated via AES-NI). If `secure: false`, stream plaintext **with conspicuous warning** (log + IPC `secure_mode_changed { enabled: false, warning: "UNENCRYPTED_STREAM" }`).
* [ ] **Certificate/PSK Management:** Support both PKI (certificate pinning) and PSK modes; PSK derived from pairing code (Phase 6).
* [ ] **IPC:** UDS credentials (`SO_PEERCRED`) — only same-UID processes may connect; optional capabilities allowlist.
* [ ] **Sandboxing:** Run core with minimal caps (`CAP_SYS_ADMIN` for DMA-BUF only via `capsh`/`systemd`); seccomp filter.
* [ ] **Input Validation:** Strict schema validation on all IPC messages; rate-limit input injection.
* [ ] **Key Rotation:** Periodic re-keying (configurable interval, default 1 hour) via ML-KEM re-encapsulation without stream interruption.

---

## 🐍 Phase 6: Python API & Frontend Assembly

*Goal: Accessible Python wrapper and GUI for end users.*

* [ ] **FastAPI Signaling Server:** WebSocket handshake (WAN pairing codes), clipboard sync, session metadata; talks to core via IPC.
* [ ] **PyQt6/PySide6 GUI:** System tray, config (bitrate, codec, encoder), connection list, stats overlay.
* [ ] **Process Management:** Launch/monitor core via `asyncio.subprocess` with socket activation; auto-restart on crash; IPC health checks.
* [ ] **Packaging:** `pyinstaller`/`cargo-bundle` for standalone distributable; systemd user unit template.