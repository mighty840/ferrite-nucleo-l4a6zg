# ferrite-nucleo-l4a6zg

Example firmware demonstrating the [ferrite-sdk](https://github.com/mighty840/ferrite-sdk)
on an STM32 Nucleo-L4A6ZG (Nucleo-144) board.

The firmware provisions a device key, collects metrics (`loop_count`, `uptime_seconds`),
encodes heartbeat and metric chunks, and streams them over RTT to the host. A Python
bridge script forwards the chunks to the ferrite-server for display on the dashboard.

## Hardware

| Item | Value |
|------|-------|
| Board | [NUCLEO-L4A6ZG](https://www.st.com/en/evaluation-tools/nucleo-l4a6zg.html) |
| MCU | STM32L4A6ZGTx — Cortex-M4F, 1 MB Flash, 320 KB RAM |
| Target | `thumbv7em-none-eabihf` |
| Debug probe | Onboard ST-LINK/V2.1 |
| LD1 (green, PC7) | 1 Hz heartbeat (100 ms on, 900 ms off) |
| LD2 (blue, PB7) | Triple flash on boot; blinks during chunk encoding |

## Prerequisites

```bash
# Rust embedded toolchain
rustup target add thumbv7em-none-eabihf

# probe-rs for flashing and RTT
cargo install probe-rs-tools

# Python bridge dependencies
pip install requests pyserial
```

**USB permissions (Linux):** If `probe-rs` fails with "Permission denied", add udev
rules for ST-LINK:

```bash
# /etc/udev/rules.d/69-probe-rs.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="0483", ATTR{idProduct}=="374b", MODE="0666"
```

Then reload: `sudo udevadm control --reload-rules && sudo udevadm trigger`

## Quick start

```bash
# 1. Clone (ferrite-sdk must be at ../iotai_sdk/ferrite-sdk)
git clone https://github.com/mighty840/ferrite-nucleo-l4a6zg.git
cd ferrite-nucleo-l4a6zg

# 2. Build
cargo build --release

# 3. Flash and run (shows defmt logs via RTT)
cargo run --release

# 4. In another terminal, start ferrite-server
cd ../iotai_sdk && cargo run -p ferrite-server

# 5. Run the RTT bridge to forward chunks to the server
python3 rtt_bridge.py
```

## Project layout

```
src/main.rs      Firmware entry point (embassy async, no_std)
memory.x         Linker script: Flash, RAM, and retained-RAM regions
build.rs         Copies memory.x to OUT_DIR, generates FERRITE_BUILD_ID
rtt_bridge.py    Host-side script: RTT capture -> HTTP POST to ferrite-server
.cargo/config.toml   Build target, probe-rs runner, defmt linker flags
```

## How it works

1. **Boot** — embassy initializes the MCU at 4 MHz (MSI default). Blue LED flashes 3 times.
2. **SDK init** — ferrite-sdk is configured with a device ID, firmware version, and a
   retained-RAM region at the top of SRAM for persisting data across resets.
3. **Device key** — `provision_device_key(0xA3, seed)` creates a 32-bit key
   (`A3xxxxxx`) stored in retained RAM. The key survives resets and identifies the
   device in every heartbeat.
4. **Main loop** (1 Hz) — each iteration:
   - Blinks the green LED
   - Increments `loop_count` (counter metric)
   - Sets `uptime_seconds` (gauge metric) to the elapsed seconds
   - Encodes a heartbeat chunk (34 bytes: device key, uptime, metric count)
   - Every 5 s, encodes a metric chunk with all current metric values
   - Logs chunks as hex over RTT (`defmt::info!("CHUNK:...")`)
5. **Bridge** — `rtt_bridge.py` runs `probe-rs run`, parses `CHUNK:[...]` lines from
   the RTT output, decodes the hex into binary, and POSTs each chunk to the
   ferrite-server's `/ingest/chunks` endpoint.

## RTT bridge usage

```bash
# Default: localhost:4000, admin/admin auth
python3 rtt_bridge.py

# Custom server
python3 rtt_bridge.py --server http://192.168.1.100:4000

# Custom auth
python3 rtt_bridge.py --user myuser --password mypass
```

The bridge flashes the firmware, captures RTT output, and forwards chunks in real time.
Press Ctrl+C to stop.

## Registering the device on the server

Before the dashboard shows the device by name, register it:

```bash
curl -u admin:admin -X POST http://localhost:4000/devices/register \
  -H "Content-Type: application/json" \
  -d '{"device_key": "A3B15569", "name": "Nucleo-L4A6ZG", "tags": "stm32,nucleo"}'
```

The device key is logged at boot (`boot ok, device_key=a3b15569`). Once registered,
incoming heartbeats with that key automatically set the device status to "online".

## Known issues and pitfalls

### Embassy version pinning

The ferrite-sdk depends on `embassy-time 0.3`. This firmware must use compatible
embassy crate versions:

| Crate | Version | Notes |
|-------|---------|-------|
| embassy-stm32 | 0.1 | Only version compatible with embassy-time 0.3 |
| embassy-executor | 0.5 | Must match embassy-stm32 0.1's expectations |
| embassy-time | 0.3 | Pinned by ferrite-sdk |

Using newer embassy versions (0.3+, 0.7+) causes `embassy-time-driver` linker conflicts.

### UART initialization order

If you add UART back to the firmware, initialize it **before** `ferrite_sdk::init()`.
Both LPUART and ferrite-sdk use critical sections internally, and initializing the SDK
first causes a panic inside `Uart::new()` due to a nested critical section in
`enable_and_reset()`.

### UploadManager stack overflow

The SDK's `UploadManager::upload_async()` allocates ~8 KB on the stack
(`heapless::Vec<heapless::Vec<u8, 256>, 32>`). This overflows the default Cortex-M
stack. The firmware works around this by calling `state.encoder.encode_heartbeat()`
directly with a single `[u8; 256]` buffer.

### VCP serial port

The Nucleo-L4A6ZG's ST-LINK VCP is wired to LPUART1 (PG7 TX / PG8 RX), but
embassy-stm32 0.1 panics when configuring LPUART1 at 115200 baud with the default
4 MHz MSI clock (BRR out of range). At 9600 baud it doesn't panic, but some board
revisions have unpopulated solder bridges that prevent VCP data from reaching the host.

**The RTT bridge avoids this entirely** by using the debug probe (SWD) for data
transport instead of serial. This is more reliable and doesn't depend on VCP hardware
configuration.

### defmt version mismatch

`defmt 0.3.100` is a shim for `defmt 1.0`. Newer versions of `probe-rs` may fail to
parse the defmt section format. If you see defmt decoding errors, pin `defmt = "=0.3.10"`
in Cargo.toml.

### memory.x and the `memory-x` feature

Do **not** enable the `memory-x` feature on `embassy-stm32`. It provides a generic
memory.x that conflicts with the custom one in this repo (which adds the `RETAINED`
section for ferrite-sdk's retained RAM). The `build.rs` copies our `memory.x` to
`OUT_DIR` so the cortex-m-rt linker script finds it.

## License

Same license as ferrite-sdk.
