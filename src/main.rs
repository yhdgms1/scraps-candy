use anyhow::Result;
use dxgi_capture_rs::{CaptureError, DXGIManager};
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum CircleSearchState {
    LookingForOpeningGrayCircle,
    LookingForOpeningYellowCircle(usize),
    LookingForOpeningRedCircle,
    LookingForMiddleGrayCircle,
    LookingForClosingYellowCircle,
    LookingForClosingRedCircle,
    LookingForClosingGrayCircle,
    Found,
}

impl Default for CircleSearchState {
    fn default() -> Self {
        Self::LookingForOpeningGrayCircle
    }
}

impl CircleSearchState {
    fn is_gray((r, g, b): (u8, u8, u8)) -> bool {
        r == 0x33 && g == 0x33 && b == 0x33
    }

    fn is_yellow((r, g, b): (u8, u8, u8)) -> bool {
        r > 0x9c && g > 0x9c && b < 0x1f
    }

    fn is_red((r, g, b): (u8, u8, u8)) -> bool {
        r > 0x9c && g < 0x1f && b < 0x1f
    }

    fn next(self, (r, g, b): (u8, u8, u8)) -> Self {
        match self {
            Self::LookingForOpeningGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForOpeningYellowCircle(0)
                } else {
                    self
                }
            }
            Self::LookingForOpeningYellowCircle(len) => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForOpeningYellowCircle(len + 1)
                } else if len > 10 && Self::is_yellow((r, g, b)) {
                    Self::LookingForOpeningRedCircle
                } else {
                    self
                }
            }
            Self::LookingForOpeningRedCircle => {
                if Self::is_red((r, g, b)) {
                    Self::LookingForMiddleGrayCircle
                } else {
                    self
                }
            }
            Self::LookingForMiddleGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForClosingRedCircle
                } else {
                    self
                }
            }
            Self::LookingForClosingRedCircle => {
                if Self::is_red((r, g, b)) {
                    Self::LookingForClosingYellowCircle
                } else {
                    self
                }
            }
            Self::LookingForClosingYellowCircle => {
                if Self::is_yellow((r, g, b)) {
                    Self::LookingForClosingGrayCircle
                } else {
                    self
                }
            }
            Self::LookingForClosingGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::Found
                } else {
                    self
                }
            }
            Self::Found => self,
        }
    }
}

#[derive(PartialEq)]
enum LineSearchState {
    LookingForLightestCorner,
    LookingForWideMatchingZoneStart,
    LookingForNarrowMatchingZoneStart,
    LookingForArrow,
    LookingForNarrowMatchingZoneEnd,
    LookingForWideMatchingZoneEnd,
    LookingForDarkestCorner,
    Found,
}

impl Default for LineSearchState {
    fn default() -> Self {
        Self::LookingForLightestCorner
    }
}

impl LineSearchState {
    fn is_narrow_matching_zone((r, g, b): (u8, u8, u8)) -> bool {
        r > 0x9c && g > 0x9c && b < 0x1f
    }

    fn is_wide_matching_zone((r, g, b): (u8, u8, u8)) -> bool {
        r < 0xfb && r > 0x91 && g < 0xfb && g > 0x91 && b < 0xfb && b > 0x91
    }

    fn next(self, (r, g, b): (u8, u8, u8)) -> Self {
        match self {
            Self::LookingForLightestCorner => {
                if r == 0x2f && g == 0x2f && b == 0x2f {
                    Self::LookingForWideMatchingZoneStart
                } else {
                    self
                }
            }
            Self::LookingForWideMatchingZoneStart => {
                if Self::is_wide_matching_zone((r, g, b)) {
                    Self::LookingForNarrowMatchingZoneStart
                } else {
                    self
                }
            }
            Self::LookingForNarrowMatchingZoneStart => {
                if Self::is_narrow_matching_zone((r, g, b)) {
                    Self::LookingForArrow
                } else {
                    self
                }
            }
            Self::LookingForArrow => {
                if r == 0xff && g == 0x57 && b == 0x5a {
                    Self::LookingForNarrowMatchingZoneEnd
                } else {
                    self
                }
            }
            Self::LookingForNarrowMatchingZoneEnd => {
                if Self::is_narrow_matching_zone((r, g, b)) {
                    Self::LookingForWideMatchingZoneEnd
                } else {
                    self
                }
            }
            Self::LookingForWideMatchingZoneEnd => {
                if Self::is_wide_matching_zone((r, g, b)) {
                    Self::LookingForDarkestCorner
                } else {
                    self
                }
            }
            Self::LookingForDarkestCorner => {
                if r == 0x19 && g == 0x19 && b == 0x19 {
                    Self::Found
                } else {
                    self
                }
            }
            Self::Found => self,
        }
    }
}

fn main() -> Result<()> {
    ThreadPoolBuilder::new()
        .num_threads(4)
        .build_global()
        .unwrap();

    let mut enigo = Enigo::new(&EnigoSettings::default()).unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .unwrap();

    let mut manager = DXGIManager::new(5)?;

    println!("Ctrl+C to stop.");

    let mut i: u64 = 0;
    let mut last_fired: Option<Instant> = None;

    while running.load(Ordering::SeqCst) {
        match manager.capture_frame() {
            Ok((pixels, (w, h))) => {
                i += 1;

                if i % 30 == 0 {
                    print!("\rFrame: {}", i);
                    use std::io::{self, Write};
                    let _ = io::stdout().flush();
                }

                if let Some(last) = last_fired {
                    if last.elapsed() < Duration::from_millis(1000) {
                        continue;
                    }

                    last_fired = None;
                }

                let raw: &[u8] = unsafe {
                    std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4)
                };

                let stride = w * 4;

                let get_rgb = |offset: usize| -> (u8, u8, u8) {
                    (raw[offset + 2], raw[offset + 1], raw[offset])
                };

                let found = AtomicBool::new(false);

                (0..h)
                    .step_by(25)
                    .collect::<Vec<usize>>()
                    .par_iter()
                    .any(|&y| {
                        if found.load(Ordering::Relaxed) {
                            return false;
                        }

                        let mut search_state_circle = CircleSearchState::default();
                        let mut search_state_line = LineSearchState::default();
                        let mut x = 0;

                        while x < w {
                            let offset = y * stride + x * 4;

                            if offset + 3 >= raw.len() {
                                break;
                            }

                            let (r, g, b) = get_rgb(offset);

                            search_state_circle =
                                CircleSearchState::next(search_state_circle, (r, g, b));
                            search_state_line = LineSearchState::next(search_state_line, (r, g, b));

                            if let CircleSearchState::Found = search_state_circle {
                                println!("\rНайден круг");

                                found.store(true, Ordering::Relaxed);
                                return true;
                            }

                            if let LineSearchState::Found = search_state_line {
                                println!("\rНайдена линия");

                                found.store(true, Ordering::Relaxed);
                                return true;
                            }

                            // У круга минимальный шаг 35. Рассчитывается как длина искомого элемента / 2 чтобы уж точно попасть на него
                            let step = match search_state_line {
                                LineSearchState::LookingForLightestCorner => 8,
                                _ => 1,
                            };

                            x += step;
                        }

                        return false;
                    });

                if found.load(Ordering::Acquire) {
                    last_fired = Some(Instant::now());

                    let _ = enigo.key(Key::Space, Direction::Press);
                    thread::sleep(Duration::from_millis(30));
                    let _ = enigo.key(Key::Space, Direction::Release);
                }
            }

            Err(CaptureError::Timeout) => {
                thread::sleep(Duration::from_millis(1));
            }

            Err(e) => {
                eprintln!("Capture error: {:?}", e);

                if matches!(e, CaptureError::AccessLost) {
                    println!("Access lost — recreating session...");
                    manager = DXGIManager::new(5)?;
                }

                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    println!("\nStopped.");
    Ok(())
}
