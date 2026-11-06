//! The buzzer on GPIO 6.

use embassy_rp::Peri;
use embassy_rp::clocks::clk_sys_freq;
use embassy_rp::peripherals::{PIN_6, PWM_SLICE3};
use embassy_rp::pwm::{Config, Pwm};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};

/// The tone frequency in Hz.
const TONE_HZ: u32 = 4_000;
/// The duration of one beep.
const BEEP_TIME: Duration = Duration::from_millis(100);

/// Send a signal here to make one beep. The sender does not wait.
pub static BEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
pub async fn beeper_task(slice: Peri<'static, PWM_SLICE3>, pin: Peri<'static, PIN_6>) {
    let (divider, top) = tone_timing(clk_sys_freq());
    let mut config = Config::default();
    config.divider = divider.into();
    config.top = top;
    config.compare_a = 0;
    let mut pwm = Pwm::new_output_a(slice, pin, config.clone());

    loop {
        BEEP.wait().await;
        // A duty cycle of 25 % gives a tone that is not too loud.
        config.compare_a = top / 4;
        pwm.set_config(&config);
        Timer::after(BEEP_TIME).await;
        config.compare_a = 0;
        pwm.set_config(&config);
    }
}

/// Gives the PWM divider and the `top` value for `TONE_HZ`.
///
/// The PWM frequency is `clk / (divider * (top + 1))`. The function uses
/// the smallest integer divider that lets `top` fit in 16 bits. Thus the
/// tone stays at `TONE_HZ` at each system clock.
fn tone_timing(clk: u32) -> (u8, u16) {
    for divider in 1..=u8::MAX {
        let top = clk
            .checked_div(TONE_HZ.saturating_mul(u32::from(divider)))
            .and_then(|count| count.checked_sub(1))
            .and_then(|top| u16::try_from(top).ok());
        if let Some(top) = top {
            return (divider, top);
        }
    }
    // With divider 255, each u32 clock gives a top below 4300. Thus the
    // loop always returns, and this line does not run.
    (u8::MAX, u16::MAX)
}
