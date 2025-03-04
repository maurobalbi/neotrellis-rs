#![no_std]

mod error;

use core::convert::TryFrom;

pub use crate::error::Error;
use embedded_hal::blocking::delay::{DelayMs, DelayUs};
use embedded_hal::blocking::i2c::{Read, Write};
use num_enum::{IntoPrimitive, TryFromPrimitive};

use core::convert::TryInto;

#[derive(Clone, Copy)]
pub struct Color {
  pub r: u8,
  pub g: u8,
  pub b: u8,
}

impl Color {
  pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
    Color { r, g, b }
  }

  pub const fn as_grb_slice(&self) -> [u8; 3] {
    [self.g, self.r, self.b]
  }
}

#[repr(u8)]
#[derive(TryFromPrimitive, IntoPrimitive, Clone, Copy)]
pub enum Event {
  High = 0,
  Low = 1,
  Falling = 2,
  Rising = 3,
}
#[derive(Clone, Copy)]
pub struct KeypadEvent {
  pub key: Key,
  pub event: Event,
}

#[derive(Clone, Copy)]
pub struct MultiEvent {
  pub coordinate: (u8, u8),
  pub event: Event,
}

pub struct MultiTrellis<'a, I2C>
where
  I2C: Write + Read,
{
  pub trellis: &'a mut [&'a mut [NeoTrellis<I2C>]],
}

pub struct NeoTrellis<I2C>
where
  I2C: Write + Read,
{
  bus: I2C,
  address: u8,
}

#[derive(Clone, Copy)]
pub struct Key(u8);

impl Key {
  pub fn deserialize(wire_byte: u8) -> Self {
    Self(((wire_byte & 0xf8) >> 1) | (wire_byte & 0x03))
  }

  pub fn serialize(&self) -> u8 {
    ((self.0 & 0xC) << 1) | (self.0 & 0x03)
  }

  pub fn index(&self) -> u8 {
    self.0
  }

  pub fn from_index(index: u8) -> Self {
    Self(index)
  }
}

#[repr(u8)]
#[derive(IntoPrimitive)]
enum Module {
  Status = 0x00,
  Neopixel = 0x0E,
  Keypad = 0x10,
}

const STATUS_HW_ID: u8 = 0x01;
const STATUS_SWRST: u8 = 0x7f;

const NEOPIXEL_PIN: u8 = 0x01;
const _NEOPIXEL_SPEED: u8 = 0x02;
const NEOPIXEL_BUF_LENGTH: u8 = 0x03;
const NEOPIXEL_BUF: u8 = 0x04;
const NEOPIXEL_SHOW: u8 = 0x05;

const _KEYPAD_STATUS: u8 = 0x00;
const KEYPAD_EVENT: u8 = 0x01;
const _KEYPAD_INTENSET: u8 = 0x02;
const _KEYPAD_INTENCLR: u8 = 0x03;
const KEYPAD_COUNT: u8 = 0x04;
const KEYPAD_FIFO: u8 = 0x10;

const HW_ID_CODE: u8 = 0x55;

impl<'a, I2> MultiTrellis<'a, I2>
where
  I2: Read + Write,
  <I2 as Read>::Error: core::fmt::Debug,
  <I2 as Write>::Error: core::fmt::Debug,
{
  pub fn set_led_color<DELAY: DelayMs<u32> + DelayUs<u32>>(
    &mut self,
    index: (u8, u8),
    color: Color,
    delay: &mut DELAY,
  ) -> Result<(), Error<I2>> {
    let (x, y) = index;

    let tx = usize::from(x / 4);
    let ty = usize::from(y / 4);

    let i = x % 4 + (y % 4) * 4;

    if tx < self.trellis.len() && ty < self.trellis[tx].len() {
      self.trellis[tx][ty].set_led_color(i, color, delay)?;
    }

    Ok(())
  }

  pub fn show<DELAY: DelayUs<u32>>(&mut self, delay: &mut DELAY) -> Result<(), Error<I2>> {
    for row in self.trellis.iter_mut() {
      for trellis in row.iter_mut() {
        trellis.show_led(delay)?
      }
    }

    Ok(())
  }

  pub fn read_events<DELAY: DelayMs<u32>>(
    &mut self,
    events: &mut [Option<MultiEvent>],
    delay: &mut DELAY,
  ) -> Result<(), Error<I2>> {
    
    for (x, row) in self.trellis.iter_mut().enumerate() {
      for (y, trellis) in row.iter_mut().enumerate() {
        let mut single_event = [None; 16];
        trellis.read_key_events(&mut single_event, delay)?;

        for e in single_event {
          let xc: u8= x.try_into().unwrap();
          let yc: u8 = y.try_into().unwrap();
          match e {
            Some(KeypadEvent { key, event }) => {
              events[x + 4 * y] = Some(MultiEvent {
                coordinate: (4 * xc + key.index() % 4, 4 * yc + key.index() / 4),
                event
              })
            },
            _ => ()
          }
        }
      }
    }

    Ok(())
  }
}

impl<I2C> NeoTrellis<I2C>
where
  I2C: Read + Write,
  <I2C as Read>::Error: core::fmt::Debug,
  <I2C as Write>::Error: core::fmt::Debug,
{
  pub fn new<DELAY: DelayMs<u32>>(
    bus: I2C,
    address: u8,
    delay: &mut DELAY,
  ) -> Result<Self, Error<I2C>> {
    let mut neotrellis = Self { bus, address };

    neotrellis.soft_reset(delay)?;
    neotrellis.setup_neopixel()?;
    neotrellis.setup_keypad()?;

    Ok(neotrellis)
  }

  fn soft_reset<DELAY: DelayMs<u32>>(&mut self, delay: &mut DELAY) -> Result<(), Error<I2C>> {
    self.write_register(Module::Status, STATUS_SWRST, &[0xff])?;
    delay.delay_ms(500);

    let mut id = [0u8];
    self.read_register(Module::Status, STATUS_HW_ID, &mut id, delay)?;

    if id[0] != HW_ID_CODE {
      Err(Error::WrongChipId)
    } else {
      Ok(())
    }
  }

  fn setup_neopixel(&mut self) -> Result<(), Error<I2C>> {
    // Set the neopixel pin
    let pin: u8 = 3;
    self.write_register(Module::Neopixel, NEOPIXEL_PIN, &pin.to_be_bytes())?;

    // We have 16 LEDs * 3 colors
    let buffer_length: u16 = 16 * 3;
    self.write_register(
      Module::Neopixel,
      NEOPIXEL_BUF_LENGTH,
      &buffer_length.to_be_bytes(),
    )?;

    Ok(())
  }

  fn setup_keypad(&mut self) -> Result<(), Error<I2C>> {
    // Enable only rising and falling edge detections for all 16 keys
    for i in 0..16 {
      let key = Key::from_index(i);
      self.set_key_event(key, Event::Low, false)?;
      self.set_key_event(key, Event::High, false)?;
      self.set_key_event(key, Event::Falling, true)?;
      self.set_key_event(key, Event::Rising, true)?;
    }

    Ok(())
  }

  pub fn set_key_event(&mut self, key: Key, event: Event, enable: bool) -> Result<(), Error<I2C>> {
    let command = (1 << (u8::from(event) + 1)) | (enable as u8);
    self.write_register(Module::Keypad, KEYPAD_EVENT, &[key.serialize(), command])?;

    Ok(())
  }

  fn read_register<DELAY: DelayMs<u32>>(
    &mut self,
    module: Module,
    register: u8,
    value: &mut [u8],
    delay: &mut DELAY,
  ) -> Result<(), Error<I2C>> {
    let command = [module.into(), register];
    self
      .bus
      .write(self.address, &command[0..2])
      .map_err(|e| Error::WriteError(e))?;

    delay.delay_ms(6u32);

    self
      .bus
      .read(self.address, value)
      .map_err(|e| Error::ReadError(e))?;

    Ok(())
  }

  fn write_register(
    &mut self,
    module: Module,
    register: u8,
    value: &[u8],
  ) -> Result<(), Error<I2C>> {
    assert!(value.len() < 32);
    let mut command = [0u8; 34];
    command[0] = module.into();
    command[1] = register;
    command[2..(2 + value.len())].copy_from_slice(value);
    self
      .bus
      .write(self.address, &command[0..(2 + value.len())])
      .map_err(|e| Error::WriteError(e))?;

    Ok(())
  }

  pub fn set_led_color<DELAY: DelayUs<u32>>(
    &mut self,
    led: u8,
    color: Color,
    delay: &mut DELAY,
  ) -> Result<(), Error<I2C>> {
    let led_address = (led as u16) * 3;
    let mut command = [0u8; 5];

    command[0..2].copy_from_slice(&led_address.to_be_bytes());
    command[2..5].copy_from_slice(&color.as_grb_slice());

    self.write_register(Module::Neopixel, NEOPIXEL_BUF, &command)?;

    delay.delay_us(100);

    Ok(())
  }

  pub fn show_led<DELAY: DelayUs<u32>>(&mut self, delay: &mut DELAY) -> Result<(), Error<I2C>> {
    self.write_register(Module::Neopixel, NEOPIXEL_SHOW, &[])?;

    delay.delay_us(100);

    Ok(())
  }

  pub fn read_key_events<DELAY: DelayMs<u32>>(
    &mut self,
    events: &mut [Option<KeypadEvent>],
    delay: &mut DELAY,
  ) -> Result<(), Error<I2C>> {
    assert!(events.len() <= 32);
    let mut buffer = [0u8; 32];
    self.read_register(
      Module::Keypad,
      KEYPAD_FIFO,
      &mut buffer[0..events.len()],
      delay,
    )?;

    for (i, item) in buffer[0..events.len()].iter().enumerate() {
      events[i] = if *item == 0xff {
        None
      } else {
        Some(KeypadEvent {
          key: Key::deserialize(item >> 2),
          event: Event::try_from(item & 0x03).unwrap(),
        })
      };
    }

    Ok(())
  }

  pub fn keypad_count<DELAY: DelayMs<u32>>(&mut self, delay: &mut DELAY) -> Result<u8, Error<I2C>> {
    let mut value = [0u8];
    self.read_register(Module::Keypad, KEYPAD_COUNT, &mut value, delay)?;

    let count = u8::from_be_bytes(value);

    Ok(count)
  }
}
