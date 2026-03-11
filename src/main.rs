#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::{Duration, Timer};
use ferrite_sdk::{RamRegion, RebootReason, SdkConfig};

use defmt_rtt as _;
use panic_probe as _;

mod build_id {
    pub fn get() -> u64 {
        env!("FERRITE_BUILD_ID").parse().unwrap_or(0)
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hsi = true;
    let p = embassy_stm32::init(config);

    let mut led_green = Output::new(p.PC7, Level::Low, Speed::Low);
    let mut led_blue = Output::new(p.PB7, Level::Low, Speed::Low);
    let mut _led_red = Output::new(p.PB14, Level::Low, Speed::Low);

    // Init ferrite-sdk
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

    let dk = ferrite_sdk::provision_device_key(0xA3, build_id::get() as u32);
    defmt::info!("Device key: {:#010X}", dk);

    if let Some(fault) = ferrite_sdk::fault::last_fault() {
        defmt::error!("Recovered from fault: PC={:#010x}", fault.frame.pc);
    }

    let reason = read_stm32l4_reset_reason();
    ferrite_sdk::reboot_reason::record_reboot_reason(reason);

    defmt::info!("Ferrite Nucleo-L4A6ZG running (no UART)");

    let mut counter: u32 = 0;
    loop {
        led_green.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led_green.set_low();
        Timer::after(Duration::from_millis(900)).await;

        counter += 1;
        let _ = ferrite_sdk::metric_increment!("loop_count");
        let _ = ferrite_sdk::metric_gauge!("uptime_seconds", counter);

        // Brief blue flash to show metrics collected
        led_blue.set_high();
        Timer::after(Duration::from_millis(50)).await;
        led_blue.set_low();

        if counter % 10 == 0 {
            defmt::info!("iteration {}", counter);
        }
    }
}

fn read_stm32l4_reset_reason() -> RebootReason {
    const RCC_CSR: *mut u32 = 0x4002_1094 as *mut u32;
    let csr = unsafe { core::ptr::read_volatile(RCC_CSR) };
    unsafe { core::ptr::write_volatile(RCC_CSR, csr | (1 << 23)) };
    match csr {
        r if r & (1 << 26) != 0 => RebootReason::HardFault,
        r if r & (1 << 27) != 0 => RebootReason::PinReset,
        r if r & (1 << 28) != 0 => RebootReason::PowerOnReset,
        r if r & (1 << 29) != 0 => RebootReason::SoftwareReset,
        r if r & (1 << 30) != 0 => RebootReason::WatchdogTimeout,
        r if r & (1 << 31) != 0 => RebootReason::WatchdogTimeout,
        _ => RebootReason::Unknown,
    }
}
