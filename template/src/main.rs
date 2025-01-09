#[cfg(feature = "debug")]
use std::io::{self, Write};

#[cfg(feature = "debug")]
use std::sync::mpsc;

#[cfg(feature = "debug")]
use std::thread;

#[cfg(feature = "debug")]
use std::time::{Duration, Instant};

#[cfg(feature = "debug")]
use termion; // デバッグビルド時のみインクルード

use lazy_static::lazy_static;
use std::sync::Mutex;

use wasm_bindgen::prelude::*;

lazy_static! {
    // 80x24 のスクリーンバッファ
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

#[cfg(feature = "debug")]
fn draw_screen() {
    let screen = SCREEN.lock().unwrap(); // バッファをロックして変更
    print!("\x1b[H\x1b[2J{}", screen.join("\n\r"));
    std::io::stdout().flush().unwrap();
}

fn main() {
    #[cfg(feature = "debug")]
    {
        use termion::raw::IntoRawMode;
        use termion::*;

        let mut stdout = io::stdout().into_raw_mode().unwrap();
        write!(stdout, "{}", clear::All).unwrap();
        stdout.flush().unwrap();

        let (tx, rx) = mpsc::channel();
        let frame_duration = Duration::from_millis(100); // 10 FPS

        // 別スレッドでキー入力を待ち受け
        let hundle = thread::spawn(move || {
            use std::io::{stdin, Read};
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
