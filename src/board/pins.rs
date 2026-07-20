//! `PineTime` pin map, derived from the public `PineTime` schematic and the
//! original `pinetime-rtic` proof of concept.

// This module is the board's pin-map reference. Not every pin is used by the
// current touch-test firmware, but keeping the complete map here avoids magic
// numbers as more drivers are enabled.
#![allow(dead_code)]

pub const DISPLAY_WIDTH: u16 = 240;
pub const DISPLAY_HEIGHT: u16 = 240;

pub const LCD_SCK: u8 = 2;
pub const LCD_MOSI: u8 = 3;
pub const LCD_MISO: u8 = 4;
pub const LCD_DC: u8 = 18;
pub const LCD_CS: u8 = 25;
pub const LCD_RESET: u8 = 26;

pub const BACKLIGHT_LOW: u8 = 14;
pub const BACKLIGHT_MID: u8 = 22;
pub const BACKLIGHT_HIGH: u8 = 23;

pub const BUTTON_INPUT: u8 = 13;
pub const BUTTON_ENABLE: u8 = 15;

pub const CHARGE_STATUS: u8 = 12;
pub const BATTERY_ADC: u8 = 31;

pub const TOUCH_SDA: u8 = 6;
pub const TOUCH_SCL: u8 = 7;
pub const TOUCH_RESET: u8 = 10;
pub const TOUCH_INTERRUPT: u8 = 28;
