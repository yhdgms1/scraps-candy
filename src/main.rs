use anyhow::Result;
use dxgi_capture_rs::{CaptureError, DXGIManager};
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum SearchState {
    LookingForOpeningGrayCircle,
    LookingForOpeningYellowCircle(usize),
    LookingForOpeningRedCircle,
    LookingForMiddleGrayCircle,
    LookingForClosingYellowCircle,
    LookingForClosingRedCircle,
    LookingForClosingGrayCircle,
    SearchComplete,
}

impl Default for SearchState {
    fn default() -> Self {
        SearchState::LookingForOpeningGrayCircle
    }
}

fn main() -> Result<()> {
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

                let is_gray =
                    |(r, g, b): (u8, u8, u8)| -> bool { r == 0x33 && g == 0x33 && b == 0x33 };

                let is_yellow =
                    |(r, g, b): (u8, u8, u8)| -> bool { r > 0x9c && g > 0x9c && b < 0x1f };

                let is_red = |(r, g, b): (u8, u8, u8)| -> bool { r > 0x9c && g < 0x1f && b < 0x1f };

                let mut found = false;

                for y in (0..h).step_by(25) {
                    let mut search_state = SearchState::default();

                    let mut x = 0;

                    while x < w {
                        let offset = y * stride + x * 4;

                        if offset + 3 >= raw.len() {
                            break;
                        }

                        let (r, g, b) = get_rgb(offset);

                        match search_state {
                            SearchState::LookingForOpeningGrayCircle => {
                                if is_gray((r, g, b)) {
                                    search_state = SearchState::LookingForOpeningYellowCircle(0);
                                }
                            }
                            SearchState::LookingForOpeningYellowCircle(len) => {
                                if is_gray((r, g, b)) {
                                    search_state =
                                        SearchState::LookingForOpeningYellowCircle(len + 1);
                                } else if len > 10 {
                                    if is_yellow((r, g, b)) {
                                        search_state = SearchState::LookingForOpeningRedCircle;
                                    }
                                }
                            }
                            SearchState::LookingForOpeningRedCircle => {
                                if is_red((r, g, b)) {
                                    search_state = SearchState::LookingForMiddleGrayCircle;
                                }
                            }
                            SearchState::LookingForMiddleGrayCircle => {
                                if is_gray((r, g, b)) {
                                    search_state = SearchState::LookingForClosingRedCircle;
                                }
                            }
                            SearchState::LookingForClosingRedCircle => {
                                if is_red((r, g, b)) {
                                    search_state = SearchState::LookingForClosingYellowCircle;
                                }
                            }
                            SearchState::LookingForClosingYellowCircle => {
                                if is_yellow((r, g, b)) {
                                    search_state = SearchState::LookingForClosingGrayCircle;
                                }
                            }
                            SearchState::LookingForClosingGrayCircle => {
                                if is_gray((r, g, b)) {
                                    search_state = SearchState::SearchComplete;
                                }
                            }
                            SearchState::SearchComplete => {
                                found = true;
                                break;
                            }
                        }

                        let step = match search_state {
                            SearchState::LookingForOpeningGrayCircle => 35,
                            _ => 1,
                        };

                        x += step;
                    }

                    if search_state == SearchState::SearchComplete {
                        break;
                    }
                }

                if found {
                    last_fired = Some(Instant::now());

                    let _ = enigo.key(Key::Space, Direction::Press);
                    thread::sleep(Duration::from_millis(30));
                    let _ = enigo.key(Key::Space, Direction::Release);

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
