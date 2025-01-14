use std::io::{stdin, stdout, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use termion::raw::IntoRawMode;
use termion::*;

use template::{game_loop, get_screen, key, Button};

/* debug logic below  */
fn draw_screen() {
    let bytes = get_screen();
    let chars: String = bytes.iter().map(|&b| b as char).collect();
    print!("\x1b[H\x1b[2J{}", chars);
    stdout().flush().unwrap();
}

fn main() {
    let mut _stdout = stdout().into_raw_mode().unwrap();

    write!(_stdout, "{}", clear::All).unwrap();
    _stdout.flush().unwrap();

    let (tx, rx) = mpsc::channel();
    let frame_duration = Duration::from_millis(33); // 30 FPS

    // 別スレッドでキー入力を待ち受け
    let _hundle = thread::spawn(move || {
        let mut buffer = [0; 1];
        while let Ok(_) = stdin().read(&mut buffer) {
            match tx.send(buffer[0] as char) {
                Ok(_) => {}
                Err(_) => break, //"Main thread is gone. Shutting down."
            }
        }
    });

    let mut running = true;
    let mut x = 0;

    while running {
        let start_time = Instant::now();

        // 入力処理（非ブロッキング）
        if let Ok(input) = rx.try_recv() {
            match input {
                'q' => running = false, // 'q'キーで終了,
                'w' => key(Button::Up),
                'a' => key(Button::Left),
                's' => key(Button::Down),
                'd' => key(Button::Right),
                _ => {}
            }
        }

        // 状態の更新（ゲームロジック）
        // game_loop();

        // drawing screen
        draw_screen();

        // フレーム間隔を維持
        let elapsed = start_time.elapsed();
        if elapsed < frame_duration {
            thread::sleep(frame_duration - elapsed);
        }
    }

    drop(rx);
}
