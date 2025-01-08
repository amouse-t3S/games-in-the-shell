use std::io::{stdout, Write};
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::*;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let mut stdout = stdout().into_raw_mode().unwrap();

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
                Ok(_) => {
                    println!("Sent {}", buffer[0]);
                    //thread::sleep(Duration::from_secs(1)); // 定期的に動作
                }
                Err(_) => {
                    println!("Main thread is gone. Shutting down.");
                    break;
                }
            }
        }
    });

    let mut screen = String::new();
    let mut running = true;

    let launch_time = Instant::now();

    while running {
        let start_time = Instant::now();

        // 入力処理（非ブロッキング）
        if let Ok(input) = rx.try_recv() {
            match input {
                'q' => running = false,  // 'q'キーで終了
                _ => screen.push(input), //format!("You pressed: {}", input),
            }
        }

        // 状態の更新（ゲームロジック）
        // game_loop();

        // 描画処理
        print!("\x1b[H\x1b[2J{}", launch_time.elapsed().as_secs());
        std::io::stdout().flush().unwrap();

        // フレーム間隔を維持
        let elapsed = start_time.elapsed();
        if elapsed < frame_duration {
            thread::sleep(frame_duration - elapsed);
        }
    }

    drop(rx);
}
