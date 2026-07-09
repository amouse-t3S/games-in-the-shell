use std::io::{stdin, stdout, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use termion::raw::IntoRawMode;
use termion::*;

use template::{game_loop, get_screen, key, reset, Button};

fn draw_screen() {
    let bytes = get_screen();
    let chars: String = bytes.iter().map(|&b| b as char).collect();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    write!(handle, "\x1b[H{}", chars).unwrap();
    handle.flush().unwrap();
}

fn main() {
    let mut _stdout = stdout().into_raw_mode().unwrap();
    write!(_stdout, "{}{}", clear::All, cursor::Hide).unwrap();
    _stdout.flush().unwrap();

    let (tx, rx) = mpsc::channel();
    let frame_duration = Duration::from_millis(67); // ~15 FPS

    let _handle = thread::spawn(move || {
        let mut buffer = [0; 1];
        while let Ok(_) = stdin().read(&mut buffer) {
            match tx.send(buffer[0] as char) {
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let mut running = true;

    while running {
        let start_time = Instant::now();

        if let Ok(input) = rx.try_recv() {
            match input {
                'q' | 'Q' => running = false,
                // Player 1: WASD
                'w' | 'W' => key(Button::Up),
                'd' | 'D' => key(Button::Right),
                's' | 'S' => key(Button::Down),
                'a' | 'A' => key(Button::Left),
                // Restart
                'r' | 'R' => reset(),
                _ => {}
            }
        }

        game_loop();
        draw_screen();

        let elapsed = start_time.elapsed();
        if elapsed < frame_duration {
            thread::sleep(frame_duration - elapsed);
        }
    }

    write!(_stdout, "{}", cursor::Show).unwrap();
    drop(rx);
}
