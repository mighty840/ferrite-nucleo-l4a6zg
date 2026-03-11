#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::usart::{self, Uart};
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_time::{Duration, Timer};
use ferrite_sdk::transport::AsyncChunkTransport;
use ferrite_sdk::upload::UploadManager;
use ferrite_sdk::{RamRegion, RebootReason, SdkConfig};

// DMA channel types for LPUART1 on STM32L4
type TxDma = peripherals::DMA2_CH6;
type RxDma = peripherals::DMA2_CH7;

use defmt_rtt as _;
use panic_probe as _;

mod build_id {
    pub fn get() -> u64 {
        env!("FERRITE_BUILD_ID").parse().unwrap_or(0)
    }
}

bind_interrupts!(struct Irqs {
    LPUART1 => usart::InterruptHandler<peripherals::LPUART1>;
});

/// Async chunk transport over LPUART1 VCP.
struct UartTransport<'a, 'd> {
    uart: &'a mut Uart<'d, peripherals::LPUART1, TxDma, RxDma>,
}

impl AsyncChunkTransport for UartTransport<'_, '_> {
    type Error = usart::Error;

    async fn send_chunk(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.uart.write(data).await
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    // Configure clocks: HSI16 -> PLL -> 80 MHz SYSCLK
    {
        use embassy_stm32::rcc::*;
        config.rcc.hsi = true;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL10,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV2), // 16 MHz * 10 / 2 = 80 MHz
        });
        config.rcc.mux = ClockSrc::PLL1_R;
    }
    let p = embassy_stm32::init(config);

    // Initialize ferrite-sdk
    ferrite_sdk::init(SdkConfig {
        device_id: "nucleo-l4a6zg-01",
        firmware_version: env!("CARGO_PKG_VERSION"),
        build_id: build_id::get(),
        ticks_fn: || embassy_time::Instant::now().as_ticks(),
        ram_regions: &[RamRegion {
            start: 0x20000000,
            end: 0x20050000, // 320 KB SRAM
        }],
    });

    // Provision device key (owner_prefix=0xA3, entropy from build ID)
    let dk = ferrite_sdk::provision_device_key(0xA3, build_id::get() as u32);
    defmt::info!(
        "Device key: {:#010X} (prefix={:#04X})",
        dk,
        (dk >> 24) as u8
    );

    // Check for previous fault
    if let Some(fault) = ferrite_sdk::fault::last_fault() {
        defmt::error!(
            "Recovered from fault: PC={:#010x} LR={:#010x}",
            fault.frame.pc,
            fault.frame.lr
        );
    }

    // Record reboot reason from RCC_CSR
    let reason = read_stm32l4_reset_reason();
    ferrite_sdk::reboot_reason::record_reboot_reason(reason);
    defmt::info!("Boot reason: {:?}", defmt::Debug2Format(&reason));

    // Set up LEDs: LD1=PC7 (green), LD2=PB7 (blue), LD3=PB14 (red)
    let mut led_green = Output::new(p.PC7, Level::Low, Speed::Low);
    let mut led_blue = Output::new(p.PB7, Level::Low, Speed::Low);
    let mut _led_red = Output::new(p.PB14, Level::Low, Speed::Low);

    // Set up LPUART1 on VCP pins: TX=PG7, RX=PG8
    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 115200;
    let mut uart = Uart::new(
        p.LPUART1,
        p.PG8, // RX
        p.PG7, // TX
        Irqs,
        p.DMA2_CH6, // TX DMA
        p.DMA2_CH7, // RX DMA
        uart_config,
    )
    .unwrap();

    defmt::info!("Ferrite Nucleo-L4A6ZG started, entering main loop");

    let mut counter: u32 = 0;

    loop {
        // Green LED heartbeat blink
        led_green.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led_green.set_low();
        Timer::after(Duration::from_millis(900)).await;

        counter += 1;
        let _ = ferrite_sdk::metric_increment!("loop_count");
        let _ = ferrite_sdk::metric_gauge!("uptime_seconds", counter);

        // Blue LED on during upload
        led_blue.set_high();
        {
            let mut transport = UartTransport { uart: &mut uart };
            match UploadManager::upload_async(&mut transport).await {
                Ok(stats) if stats.chunks_sent > 0 => {
                    defmt::info!("Uploaded {} chunks", stats.chunks_sent)
                }
                Ok(_) => {}
                Err(_e) => defmt::warn!("Upload failed"),
            }
        }
        led_blue.set_low();

        if counter % 10 == 0 {
            defmt::info!("iteration {} — metrics collected", counter);
        }
    }
}

/// Read STM32L4 reset reason from RCC_CSR register.
fn read_stm32l4_reset_reason() -> RebootReason {
    const RCC_CSR: *mut u32 = 0x4002_1094 as *mut u32;
    let csr = unsafe { core::ptr::read_volatile(RCC_CSR) };
    // Clear reset flags by setting RMVF bit (bit 23)
    unsafe { core::ptr::write_volatile(RCC_CSR, csr | (1 << 23)) };

    match csr {
        r if r & (1 << 26) != 0 => RebootReason::HardFault,       // OBLRSTF
        r if r & (1 << 27) != 0 => RebootReason::PinReset,        // PINRSTF
        r if r & (1 << 28) != 0 => RebootReason::PowerOnReset,    // BORRSTF
        r if r & (1 << 29) != 0 => RebootReason::SoftwareReset,   // SFTRSTF
        r if r & (1 << 30) != 0 => RebootReason::WatchdogTimeout, // IWDGRSTF
        r if r & (1 << 31) != 0 => RebootReason::WatchdogTimeout, // WWDGRSTF
        _ => RebootReason::Unknown,
    }
}
