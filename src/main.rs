//! Ferrite SDK example firmware for the STM32 Nucleo-L4A6ZG (Nucleo-144).
//!
//! Demonstrates device key provisioning, metric collection, and heartbeat
//! chunk encoding on a real Cortex-M4 target. Chunks are logged over RTT
//! (probe-rs) and can be forwarded to the ferrite-server using `rtt_bridge.py`.
//!
//! LEDs:
//!   - LD2 (blue, PB7): triple flash on boot, blinks during chunk encoding
//!   - LD1 (green, PC7): 1 Hz heartbeat (100 ms on, 900 ms off)

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::{Duration, Timer};
use ferrite_sdk::{RamRegion, SdkConfig};

use defmt_rtt as _;
use panic_probe as _;

mod build_id {
    pub fn get() -> u64 {
        env!("FERRITE_BUILD_ID").parse().unwrap_or(0)
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    let mut led_green = Output::new(p.PC7, Level::Low, Speed::Low);
    let mut led_blue = Output::new(p.PB7, Level::Low, Speed::Low);

    // Boot indicator: triple blue flash
    for _ in 0..3 {
        led_blue.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led_blue.set_low();
        Timer::after(Duration::from_millis(100)).await;
    }
    Timer::after(Duration::from_millis(300)).await;

    // Initialize ferrite-sdk
    ferrite_sdk::init(SdkConfig {
        device_id: "nucleo-l4a6zg-01",
        firmware_version: env!("CARGO_PKG_VERSION"),
        build_id: build_id::get(),
        ticks_fn: || embassy_time::Instant::now().as_ticks(),
        ram_regions: &[RamRegion {
            start: 0x20000000,
            end: 0x20050000,
        }],
    });

    // Provision a device key (persisted in retained RAM across resets)
    let _dk = ferrite_sdk::provision_device_key(0xA3, build_id::get() as u32);
    defmt::info!(
        "boot ok, device_key={:08x}",
        ferrite_sdk::device_key::device_key().unwrap_or(0)
    );

    // Main loop: 1 Hz heartbeat with metric collection and chunk encoding
    let mut counter: u32 = 0;
    loop {
        // Green LED heartbeat
        led_green.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led_green.set_low();
        Timer::after(Duration::from_millis(900)).await;

        counter += 1;
        let _ = ferrite_sdk::metric_increment!("loop_count");
        let _ = ferrite_sdk::metric_gauge!("uptime_seconds", counter);

        // Encode heartbeat chunk (blue LED on during encoding)
        // Uses direct encoding to avoid UploadManager's 8 KB stack allocation.
        led_blue.set_high();
        let mut buf: [u8; 256] = [0u8; 256];
        let mut len: usize = 0;
        ferrite_sdk::sdk::with_sdk(|state| {
            let uptime = ferrite_sdk::metrics::ticks();
            let dk = ferrite_sdk::device_key::device_key().unwrap_or(0);
            state.encoder.encode_heartbeat(
                uptime,
                0,
                state.metrics.len() as u32,
                state.trace.frames_lost(),
                dk,
                |chunk| {
                    buf[..chunk.len()].copy_from_slice(chunk);
                    len = chunk.len();
                },
            );
        });
        if len > 0 {
            defmt::info!("CHUNK:{=[u8]:x}", &buf[..len]);
        }

        // Encode metric chunks every 5 seconds
        if counter % 5 == 0 {
            ferrite_sdk::sdk::with_sdk(|state| {
                state.encoder.encode_metrics(state.metrics.iter(), |chunk| {
                    defmt::info!("CHUNK:{=[u8]:x}", chunk);
                });
            });
        }
        led_blue.set_low();

        if counter % 10 == 0 {
            defmt::info!("iteration {}", counter);
        }
    }
}
