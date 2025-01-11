use lazy_static::lazy_static;
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

lazy_static! {
    // 80x24 Screen buffer
    static ref SCREEN: Mutex<Vec<String>> = Mutex::new(vec!["/".repeat(80); 24]);
}

#[wasm_bindgen]
#[no_mangle]
pub extern "C" fn game_loop() {}

#[wasm_bindgen]
#[no_mangle]
pub extern "C" fn get_screen() -> Vec<u8> {
    let screen = SCREEN.lock().unwrap(); // バッファをロックしてアクセス
    let mut buffer = Vec::new();
    for row in screen.iter() {
        buffer.extend_from_slice(row.as_bytes());
        buffer.extend_from_slice("\n\r".as_bytes())
    }
    buffer
}

pub extern "C" fn update_screen(x: usize, y: usize, char: char) {
    let mut screen = SCREEN.lock().unwrap(); // バッファをロックして変更
    if y < screen.len() && x < screen[y].len() {
        let row = &mut screen[y];
        row.replace_range(x..x + 1, &char.to_string());
    }
}
