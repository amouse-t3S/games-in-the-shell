use lazy_static::lazy_static;
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

lazy_static! {
    // 80x24 Screen buffer
    static ref SCREEN: Mutex<Vec<String>> = Mutex::new(vec![".".repeat(80); 24]);

    static ref STAT: Mutex<Mem> = Mutex::new(Mem {
        keys: [0, 0, 0, 0],
        pos: Position { x: 0, y: 0 },
    });
}

struct Mem {
    keys: [u8; 4],
    pos: Position,
}

struct Position {
    x: i8,
    y: i8,
}

#[wasm_bindgen]
pub enum Button {
    Up = 1,
    Right = 2,
    Down = 4,
    Left = 8,
    North = 16,
    East = 32,
    South = 64,
    West = 128,
}

#[wasm_bindgen]
#[no_mangle]
pub fn get_screen() -> Vec<u8> {
    let screen = SCREEN.lock().unwrap();
    let joined = screen.join("\n\r");
    joined.into_bytes()
}

#[wasm_bindgen]
#[no_mangle]
pub fn key(button: Button) {
    let mut stat = STAT.lock().unwrap();
    let cx = stat.pos.x;
    let cy = stat.pos.y;
    let mut mvx: i8 = 0;
    let mut mvy: i8 = 0;
    match button {
        Button::Up => {
            if cy > 0 {
                mvy = -1;
            }
        }
        Button::Right => {
            if cx < 79 {
                mvx = 1;
            }
        }
        Button::Down => {
            if cy < 23 {
                mvy = 1;
            }
        }
        Button::Left => {
            if cx > 0 {
                mvx = -1;
            }
        }
        _ => (),
    }
    stat.pos.x = cx + mvx;
    stat.pos.y = cy + mvy;
    update_screen(stat.pos.x, stat.pos.y, 'A');
}

fn update_screen(x: i8, y: i8, char: char) {
    let x = usize::try_from(x).unwrap();
    let y = usize::try_from(y).unwrap();
    let mut screen = SCREEN.lock().unwrap();
    if y < screen.len() && x < screen[y].len() {
        let row = &mut screen[y];
        row.replace_range(x..x + 1, &char.to_string());
    }
}

#[wasm_bindgen]
#[no_mangle]
pub fn reset() {}

#[wasm_bindgen]
#[no_mangle]
pub fn game_loop() {}
