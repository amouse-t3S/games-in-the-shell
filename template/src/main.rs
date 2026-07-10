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
        let mut buf = [0u8; 1];
        loop {
            if stdin().read(&mut buf).is_err() { break; }
            let ch = buf[0];
            // Arrow keys arrive as ESC [ A/B/C/D — consume the sequence and
            // synthesise a virtual char that the main loop recognises.
            let send_ch = if ch == 0x1b {
                let mut seq = [0u8; 2];
                if stdin().read_exact(&mut seq).is_ok() && seq[0] == b'[' {
                    match seq[1] {
                        b'A' => '\x01', // up
                        b'B' => '\x02', // down
                        b'C' => '\x03', // right
                        b'D' => '\x04', // left
                        _    => continue,
                    }
                } else { continue }
            } else {
                ch as char
            };
            if tx.send(send_ch).is_err() { break; }
        }
    });

    let mut running = true;

    while running {
        let start_time = Instant::now();

        if let Ok(input) = rx.try_recv() {
            match input {
                'q' | 'Q' => running = false,
                // Player 1: WASD / hjkl / arrow keys
                'w' | 'W' | 'k' | 'K' | '\x01' => key(Button::Up),
                'd' | 'D' | 'l' | 'L' | '\x03' => key(Button::Right),
                's' | 'S' | 'j' | 'J' | '\x02' => key(Button::Down),
                'a' | 'A' | 'h' | 'H' | '\x04' => key(Button::Left),
                ' ' => key(Button::Boost),
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
