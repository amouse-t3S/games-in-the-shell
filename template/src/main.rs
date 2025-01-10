#[cfg(feature = "debug")]
use {
    std::io::{stdin, stdout, Read, Write},
    std::sync::mpsc,
    std::thread,
    std::time::{Duration, Instant},
    termion::raw::IntoRawMode,
    termion::*,
};

use lazy_static::lazy_static;
use std::sync::Mutex;

use wasm_bindgen::prelude::*;

lazy_static! {
    // 80x24 Screen buffer
    static ref SCREEN: Mutex<Vec<String>> = Mutex::new(vec!["/".repeat(80); 24]);
}

#[wasm_bindgen]
pub fn game_loop() {}

#[wasm_bindgen]
pub fn get_screen() -> Vec<u8> {
    let screen = SCREEN.lock().unwrap(); // バッファをロックしてアクセス
    let mut buffer = Vec::new();
    for row in screen.iter() {
        buffer.extend_from_slice(row.as_bytes());
    }
    buffer
}

fn update_screen(x: usize, y: usize, char: char) {
    let mut screen = SCREEN.lock().unwrap(); // バッファをロックして変更
    if y < screen.len() && x < screen[y].len() {
        let row = &mut screen[y];
        row.replace_range(x..x + 1, &char.to_string());
    }
}

/* debug logic below  */

#[cfg(feature = "debug")]
fn draw_screen() {
    let screen = SCREEN.lock().unwrap();
    print!("\x1b[H\x1b[2J{}", screen.join("\n\r"));
    stdout().flush().unwrap();
}

fn main() {
    #[cfg(feature = "debug")]
    {
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
                    'q' => running = false, // 'q'キーで終了
                    _ => {
                        update_screen(x % 80, x / 80, input);
                        x += 1;
                        x %= 80 * 24;
                    }
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
}
