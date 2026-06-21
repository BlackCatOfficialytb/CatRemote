# The Plan

## 🛠️ Phase 1: The Standalone Rust CLI (The Capture & Encode Core)

*Goal: Build a headless Rust command-line tool that grabs the Wayland desktop and encodes it in real time with zero frame copies to CPU memory.*

* [ ] **PipeWire Integration:** Use `pipewire-rs` to hook into the KDE ScreenCast Portal (`org.freedesktop.portal.ScreenCast`). Capture raw frames directly from the compositor.
* [ ] **DMA-BUF Pipeline:** Ensure the captured frames stay entirely inside VRAM as DMA-BUFs. **Zero RAM copying.**
* [ ] **Hardware Encoding Engine:** Interface directly with GPU hardware APIs using Rust bindings for **VA-API** (Intel/AMD) or **NVENC** (NVIDIA).
* [ ] **CLI Verification:** Build a basic CLI flag (e.g., `./core --record output.h264`) to test that you can dump a flawlessly encoded, hardware-accelerated H.264/HEVC stream to a local file.

---

## 🔊 Phase 2: Input Injection & Audio Loopback (The Host Core)

*Goal: Give your Rust daemon the ability to capture audio and inject user interactions back into Wayland securely.*

* [x] **Audio Capture:** Hook into **PipeWire** to create a virtual audio loopback sink. Capture system audio and encode it using a low-latency codec like Opus.
* [x] **Input Injection Core via `libei`:** Implement **`libei` (Emulated Input)** bindings in Rust. This allows your headless daemon to safely inject mouse movements, clicks, and keyboard presses into Wayland without needing root (`sudo`) access.
* [x] **Controller Mapping:** Set up a translation layer to parse generic game controller inputs (evdev/gamepad) so they can be injected cleanly on the host side.


---

## 🌐 Phase 3: The Custom Network Protocol (The Transmission Core)

*Goal: Ship the encoded video/audio chunks over the network with sub-10ms latency.*

* [ ] **The UDP/QUIC Socket Engine:** Set up an async network socket using raw UDP or the `quinn` crate (QUIC protocol) to bypass TCP's head-of-line blocking.
* [ ] **MTU Slicing & Frame Assembly:** Implement an algorithm to slice massive keyframes (I-frames) into safe MTU-sized packets for network transmission, and handle packet reassembly on the client.
* [ ] **Real-time Congestion Control:** Write a lightweight bit-rate throttling engine. If packets drop, the core must immediately signal the GPU encoder to lower the bit-rate or skip a frame to prevent latency backup.

---

## 🤝 Phase 4: Local IPC & Python Bridging

*Goal: Define how the Rust Core will talk to your future Python API and GUI layers.*

* [ ] **Local IPC Socket Server:** Implement a local Unix Domain Socket (UDS) or Named Pipe server inside the Rust core.
* [ ] **Lightweight JSON Control Protocol:** Define a simple messaging schema over the local socket so an external process can control it.
* *Example incoming message:* `{"action": "START_STREAM", "port": 8080, "codec": "hevc"}`
* *Example outgoing message:* `{"status": "STREAMING", "fps": 60, "latency_ms": 2.4}`



---

## 🐍 Phase 5: Python API & Frontend Assembly

*Goal: Wrap your elite Rust core in a highly accessible, beautiful Python application.*

* [ ] **FastAPI Signaling Server:** Write a lightweight Python backend using **FastAPI** to handle remote WebSockets connections, clipboard syncing, and connection handshakes (WAN pairing codes).
* [ ] **PyQt6 / PySide6 GUI:** Build the system tray application and configuration menus in Python.
* [ ] **Process Management:** Use Python's `subprocess` or `asyncio.create_subprocess_exec` to launch, monitor, and pass commands to your Rust core daemon over the local IPC socket.
