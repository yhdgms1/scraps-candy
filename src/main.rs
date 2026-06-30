use anyhow::Result;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use dxgi_capture_rs::{CaptureError, DXGIManager};
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
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
        CircleSearchState::LookingForOpeningGrayCircle
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

    fn next(current: Self, (r, g, b): (u8, u8, u8)) -> Self {
        match current {
            Self::LookingForOpeningGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForOpeningYellowCircle(0)
                } else {
                    current
                }
            }
            Self::LookingForOpeningYellowCircle(len) => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForOpeningYellowCircle(len + 1)
                } else if len > 10 && Self::is_yellow((r, g, b)) {
                    Self::LookingForOpeningRedCircle
                } else {
                    current
                }
            }
            Self::LookingForOpeningRedCircle => {
                if Self::is_red((r, g, b)) {
                    Self::LookingForMiddleGrayCircle
                } else {
                    current
                }
            }
            Self::LookingForMiddleGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::LookingForClosingRedCircle
                } else {
                    current
                }
            }
            Self::LookingForClosingRedCircle => {
                if Self::is_red((r, g, b)) {
                    Self::LookingForClosingYellowCircle
                } else {
                    current
                }
            }
            Self::LookingForClosingYellowCircle => {
                if Self::is_yellow((r, g, b)) {
                    Self::LookingForClosingGrayCircle
                } else {
                    current
                }
            }
            Self::LookingForClosingGrayCircle => {
                if Self::is_gray((r, g, b)) {
                    Self::Found
                } else {
                    current
                }
            }
            Self::Found => {
                current
            }
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
    .expect("Error setting Ctrl-C handler");

    let mut manager = DXGIManager::new(5)?;

    let (width, height) = manager.geometry();
    println!("DXGI capture started — {}x{}", width, height);
    println!("Press Ctrl+C to stop.");

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

                (0..h).step_by(25).collect::<Vec<usize>>().par_iter().any(|&y| {
                    if found.load(Ordering::Relaxed) {
                        return false;
                    }

                    let mut search_state = CircleSearchState::default();
                    let mut x = 0;

                    while x < w {
                        let offset = y * stride + x * 4;

                        if offset + 3 >= raw.len() {
                            break;
                        }

                        let (r, g, b) = get_rgb(offset);

                        search_state = CircleSearchState::next(search_state, (r, g, b));

                        if let CircleSearchState::Found = search_state {
                            found.store(true, Ordering::Relaxed);

                            return true;
                        }
                        
                        let step = match search_state {
                            CircleSearchState::LookingForOpeningGrayCircle => 35,
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

                    println!();
                    println!("Circle found");
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
