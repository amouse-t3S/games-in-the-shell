use lazy_static::lazy_static;
use std::collections::VecDeque;
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

const W: usize = 80;
const H: usize = 24;
const INIT_LEN: usize = 15;
const GROW_REWARD: usize = 5;        // segments gained when attacking opponent's tail
const FOOD_REWARD: usize = 2;        // segments gained when eating a food pellet
const FOOD_DROP_INTERVAL: usize = 6; // moves between food drops (snake shrinks each drop)
const SPEED_INC: u32 = 100;          // accumulator added per tick
const EFFECT_TICKS: u32 = 8;         // how long a break-effect cell stays visible
// h_thresh(len) = max(100, len*10) → horizontal period = len/10 ticks (min 1)
// v_thresh = h_thresh * 2           → vertical always half horizontal speed

const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[derive(Clone, Copy, PartialEq)]
enum Cell {
    Empty,
    Wall,
    P1Head,
    P1Body,
    P1Tail,
    P2Head,
    P2Body,
    P2Tail,
    Food,
    Effect, // temporary break-effect shown after a tail attack
}

fn cell_char(c: Cell) -> char {
    match c {
        Cell::Empty  => ' ',
        Cell::Wall   => '#',
        Cell::P1Head => '@',
        Cell::P1Body => '1',
        Cell::P1Tail => 'o',
        Cell::P2Head => '&',
        Cell::P2Body => '2',
        Cell::P2Tail => '0',
        Cell::Food   => '.',
        Cell::Effect => '*',
    }
}

struct Snake {
    body: VecDeque<(usize, usize)>,
    dir: (i32, i32),
    next_dir: (i32, i32),
    alive: bool,
    grow_pending: usize, // extra tail-pops to skip (each skipped pop = +1 length)
    move_count: usize,   // actual moves made; triggers food-drop every FOOD_DROP_INTERVAL
    h_accum: u32,        // speed accumulator for horizontal moves
    v_accum: u32,        // speed accumulator for vertical moves
    boost_ticks: u32,   // ticks remaining in the current speed burst
}

impl Snake {
    fn head(&self) -> Option<(usize, usize)> {
        self.body.front().copied()
    }
}

// Horizontal move threshold: accumulate SPEED_INC per tick, fire when >= h_thresh(len).
// Period = h_thresh / SPEED_INC ticks.  Min period = 1 tick (len ≤ 10).
// Examples: len=10→1 tick, len=15→1.5, len=20→2, len=30→3.
fn h_thresh(len: usize) -> u32 { (len as u32 * 10).max(SPEED_INC) }
fn v_thresh(len: usize) -> u32 { h_thresh(len) * 2 }

#[derive(Clone, Copy, PartialEq)]
enum Strategy { Survive, Hunt }

impl Strategy {
    fn label(self) -> &'static str {
        match self { Strategy::Survive => "SURV", Strategy::Hunt => "HUNT" }
    }
}

enum Advance {
    Still,
    Move(usize, usize),
    Crash,
}

struct Particle {
    x: usize,
    y: usize,
    ch: char,
    life: u32, // ticks remaining; removed at 0
}

impl Particle {
    fn glyph(&self) -> char {
        match self.ch {
            // Explosion particles: * → + → .
            '*' => match self.life { 1 => '.', 2 => '+', _ => '*' },
            // Wither particles: ~ → - → , → .
            '~' => match self.life { 1 => '.', 2 | 3 => ',', 4 | 5 => '-', _ => '~' },
            _   => if self.life == 1 { '.' } else { self.ch },
        }
    }
}

struct DizzyAnim {
    hx: usize,
    hy: usize,
    step: u32, // 0 = waiting for wither to clear; 1+ = active
}

struct DeathAnim {
    // Body segments queued for explosion in order, with cell type for re-painting.
    segments: VecDeque<(usize, usize, Cell)>,
    particles: Vec<Particle>,
    dizzy: Option<DizzyAnim>, // post-starvation dizzy-and-collapse phase
}

struct Game {
    board: Vec<Vec<Cell>>,
    snakes: [Snake; 2],
    over: bool,
    winner: i8,
    tick: u32,
    ai_enabled: bool,
    cpu_strategy: Strategy,
    effects: Vec<((usize, usize), u32)>, // (cell position, expiry tick)
    death_anim: Option<DeathAnim>,
    pending_winner: i8,
    boost_particles: Vec<Particle>,
}

impl Game {
    fn new() -> Self {
        let mut board = vec![vec![Cell::Empty; W]; H];
        for x in 0..W {
            board[0][x] = Cell::Wall;
            board[H - 1][x] = Cell::Wall;
        }
        for y in 0..H {
            board[y][0] = Cell::Wall;
            board[y][W - 1] = Cell::Wall;
        }

        let p1y = H / 3;
        let p1x = W / 4;
        let mut p1_body = VecDeque::new();
        for i in 0..INIT_LEN {
            p1_body.push_back((p1x.saturating_sub(i), p1y));
        }

        let p2y = H * 2 / 3;
        let p2x = W * 3 / 4;
        let mut p2_body = VecDeque::new();
        for i in 0..INIT_LEN {
            p2_body.push_back(((p2x + i).min(W - 2), p2y));
        }

        let mut game = Game {
            board,
            snakes: [
                Snake { body: p1_body, dir: (1, 0), next_dir: (1, 0), alive: true, grow_pending: 0, move_count: 0,
                    h_accum: h_thresh(INIT_LEN) - 1, v_accum: v_thresh(INIT_LEN) - 1, boost_ticks: 0 },
                Snake { body: p2_body, dir: (-1, 0), next_dir: (-1, 0), alive: true, grow_pending: 0, move_count: 0,
                    h_accum: h_thresh(INIT_LEN) - 1, v_accum: v_thresh(INIT_LEN) - 1, boost_ticks: 0 },
            ],
            over: false,
            winner: -1,
            tick: 0,
            ai_enabled: true,
            cpu_strategy: Strategy::Hunt,
            effects: Vec::new(),
            death_anim: None,
            pending_winner: -1,
            boost_particles: Vec::new(),
        };
        game.draw_snakes();
        game
    }

    // --- AI for P2 ---

    fn ai_step(&mut self) {
        if !self.ai_enabled || !self.snakes[1].alive { return; }

        let strategy = self.pick_strategy();
        self.cpu_strategy = strategy;

        let p2_ht = h_thresh(self.snakes[1].body.len());
        let can_h = self.snakes[1].h_accum >= p2_ht;
        let can_v = self.snakes[1].v_accum >= p2_ht * 2;
        if !can_h && !can_v { return; }

        let Some((hx, hy)) = self.snakes[1].head() else { return; };
        let (cdx, cdy) = self.snakes[1].dir;
        // Always allow the AI to choose a vertical direction, even when v_accum
        // isn't ready yet. try_advance will stall the move until the accumulator
        // fires — the snake pauses for a tick rather than charging into a corner.
        let allow_vert = true;

        let p2_len  = self.snakes[1].body.len() as i32;
        let p1_head = self.snakes[0].head();
        let p1_tail = self.snakes[0].body.back().copied();
        let p1_dir  = self.snakes[0].dir;

        // Project P1's head forward: in one CPU move period P1 moves
        // p2_ht / p1_ht times (ratio of their thresholds).
        let p1_ht = h_thresh(self.snakes[0].body.len());
        let p1_steps = (p2_ht / p1_ht).max(1) as i32;
        let p1_proj = p1_head.map(|(phx, phy)| {
            let (pdx, pdy) = p1_dir;
            let fx = (phx as i32 + pdx * p1_steps).clamp(1, W as i32 - 2) as usize;
            let fy = (phy as i32 + pdy * p1_steps).clamp(1, H as i32 - 2) as usize;
            (fx, fy)
        });

        let mut best_dir = (cdx, cdy);
        let mut best_score = i32::MIN;

        for &(dx, dy) in &DIRS {
            if dx == -cdx && dy == -cdy { continue; }
            if dx != 0 && !can_h    { continue; }
            if dy != 0 && !allow_vert { continue; }

            let nx = hx as i32 + dx;
            let ny = hy as i32 + dy;
            if nx <= 0 || ny <= 0 || nx >= W as i32 - 1 || ny >= H as i32 - 1 { continue; }
            let (nx, ny) = (nx as usize, ny as usize);

            let score = match self.board[ny][nx] {
                Cell::Wall | Cell::P1Head | Cell::P2Head
                | Cell::P1Body | Cell::P2Body | Cell::P2Tail => continue,
                Cell::P1Tail => 1_000_000, // attack target
                Cell::Empty | Cell::Food | Cell::Effect => {
                    // --- Universal safety layer ---

                    // Accurate empty-space count (own body NOT included — it is a real
                    // obstacle until the tail retreats past each segment).
                    let space = self.flood_fill(nx, ny) as i32;

                    // Entering a region smaller than own body is near-certain death.
                    let pocket_penalty = if space < p2_len {
                        -500_000 + space * 1_000
                    } else { 0 };

                    // Penalise cells boxed in by own body (corridors / dead ends).
                    let body_adj = DIRS.iter().filter(|&&(ddx, ddy)| {
                        let bx = nx as i32 + ddx;
                        let by = ny as i32 + ddy;
                        bx > 0 && by > 0 && bx < W as i32 - 1 && by < H as i32 - 1
                            && matches!(self.board[by as usize][bx as usize],
                                Cell::P2Body | Cell::P2Tail)
                    }).count() as i32;
                    let body_penalty = body_adj * 400;

                    // Forward exits (excluding reverse): 0 = certain death, 1 = risky.
                    let fwd_exits = DIRS.iter().filter(|&&(ddx, ddy)| {
                        if ddx == -dx && ddy == -dy { return false; }
                        let ex = nx as i32 + ddx;
                        let ey = ny as i32 + ddy;
                        ex > 0 && ey > 0 && ex < W as i32 - 1 && ey < H as i32 - 1
                            && !matches!(self.board[ey as usize][ex as usize],
                                Cell::Wall | Cell::P1Body
                                | Cell::P2Body | Cell::P2Tail | Cell::P2Head)
                    }).count();
                    let dead_end_penalty = match fwd_exits {
                        0 => 300_000,
                        1 =>  20_000,
                        _ =>       0,
                    };

                    // Penalise moving toward projected P1 head (head-on danger).
                    let head_danger = p1_proj.map(|(phx, phy)| {
                        let d = (nx as i32 - phx as i32).abs()
                              + (ny as i32 - phy as i32).abs();
                        match d { 0 => 800, 1 => 500, 2 => 150, _ => 0 }
                    }).unwrap_or(0);

                    // Small bonus for continuing current direction (reduces jitter).
                    let continuation = if (dx, dy) == (cdx, cdy) { 25 } else { 0 };

                    // --- Strategy objective (same space weight for both) ---
                    let p2_tail = self.snakes[1].body.back().copied();
                    let objective = match strategy {
                        Strategy::Survive => {
                            // Follow own tail to stay in a loop.
                            // Bonus activates within 10 cells of own tail tip; max 400.
                            let self_tail_d = p2_tail.map(|(tx, ty)| {
                                (nx as i32 - tx as i32).abs()
                                    + (ny as i32 - ty as i32).abs()
                            }).unwrap_or(999);
                            let tail_bonus = (10 - self_tail_d).max(0) * 40;
                            space * 80 - head_danger + tail_bonus + continuation
                        }
                        Strategy::Hunt => {
                            // Chase P1's tail; food is a useful bonus when nearby.
                            let attack = p1_tail.map(|(tx, ty)| {
                                let d = (nx as i32 - tx as i32).abs()
                                      + (ny as i32 - ty as i32).abs();
                                (20 - d).max(0) * 25 // max 500
                            }).unwrap_or(0);
                            let food_bonus = if self.board[ny][nx] == Cell::Food { 150 } else { 0 };
                            space * 80 + attack + food_bonus - head_danger + continuation
                        }
                    };
                    objective + pocket_penalty - body_penalty - dead_end_penalty
                }
            };

            if score > best_score {
                best_score = score;
                best_dir = (dx, dy);
            }
        }

        self.snakes[1].next_dir = best_dir;
    }

    fn pick_strategy(&self) -> Strategy {
        let Some(p2h) = self.snakes[1].head() else { return Strategy::Hunt; };
        let p2_len = self.snakes[1].body.len();
        let slower  = h_thresh(p2_len) > h_thresh(self.snakes[0].body.len());
        let p2_space = self.flood_fill(p2h.0, p2h.1) as i32;
        // Wider safety margin when slower: less ability to escape tight spots.
        let safe_thr = if slower { 50 } else { 35 };
        if p2_space < safe_thr { Strategy::Survive } else { Strategy::Hunt }
    }

    fn flood_fill(&self, sx: usize, sy: usize) -> usize {
        let mut visited = vec![false; W * H];
        let mut queue = VecDeque::new();
        visited[sy * W + sx] = true;
        queue.push_back((sx, sy));
        let mut count = 0;

        while let Some((x, y)) = queue.pop_front() {
            count += 1;
            for &(dx, dy) in &DIRS {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx <= 0 || ny <= 0 || nx >= W as i32 - 1 || ny >= H as i32 - 1 { continue; }
                let (nx, ny) = (nx as usize, ny as usize);
                let idx = ny * W + nx;
                if visited[idx] { continue; }
                match self.board[ny][nx] {
                    Cell::Empty | Cell::P1Tail | Cell::Food | Cell::Effect => {
                        visited[idx] = true;
                        queue.push_back((nx, ny));
                    }
                    _ => {}
                }
            }
        }
        count
    }

    // --- Game logic ---

    fn clear_snakes(&mut self) {
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                match self.board[y][x] {
                    Cell::P1Head | Cell::P1Body | Cell::P1Tail
                    | Cell::P2Head | Cell::P2Body | Cell::P2Tail => {
                        self.board[y][x] = Cell::Empty;
                    }
                    _ => {}
                }
            }
        }
    }

    fn draw_snakes(&mut self) {
        self.clear_snakes();
        for (pi, snake) in self.snakes.iter().enumerate() {
            if !snake.alive { continue; }
            let len = snake.body.len();
            for (i, &(x, y)) in snake.body.iter().enumerate() {
                if y >= H || x >= W { continue; }
                let cell = if i == 0 {
                    if pi == 0 { Cell::P1Head } else { Cell::P2Head }
                } else if i == len - 1 {
                    if pi == 0 { Cell::P1Tail } else { Cell::P2Tail }
                } else {
                    if pi == 0 { Cell::P1Body } else { Cell::P2Body }
                };
                self.board[y][x] = cell;
            }
        }
    }

    fn step(&mut self) {
        if self.over { return; }

        // Death animation in progress: advance it and skip normal game logic.
        if self.death_anim.is_some() {
            self.advance_death_anim();
            return;
        }

        // Expire break-effect cells that have been visible long enough.
        self.effects.retain(|&((ex, ey), expiry)| {
            if self.tick >= expiry {
                if matches!(self.board[ey][ex], Cell::Effect) {
                    self.board[ey][ex] = Cell::Empty;
                }
                false
            } else { true }
        });

        self.tick = self.tick.wrapping_add(1);
        for snake in self.snakes.iter_mut() {
            if snake.alive {
                let len = snake.body.len();
                // Cap each accumulator at threshold+SPEED_INC-1 so a long run in one
                // direction never queues up many rapid moves when the snake turns.
                snake.h_accum = snake.h_accum.saturating_add(SPEED_INC)
                    .min(h_thresh(len) + SPEED_INC - 1);
                snake.v_accum = snake.v_accum.saturating_add(SPEED_INC)
                    .min(v_thresh(len) + SPEED_INC - 1);
            }
        }
        self.ai_step();

        // Lock directions (prevent 180° reversal; direction frozen during boost)
        for snake in self.snakes.iter_mut() {
            if !snake.alive || snake.boost_ticks > 0 { continue; }
            let (ndx, ndy) = snake.next_dir;
            let (cdx, cdy) = snake.dir;
            if !(ndx == -cdx && ndy == -cdy) {
                snake.dir = snake.next_dir;
            }
        }

        let adv = [self.try_advance(0), self.try_advance(1)];

        let mut died           = [false; 2];
        let mut attacked       = [false; 2];
        let mut ate_food       = [false; 2];
        let mut starved        = [false; 2];
        let mut starvation_head: [Option<(usize, usize)>; 2] = [None; 2];

        // Head-to-head: both snakes move to the same cell
        if let (Advance::Move(x0, y0), Advance::Move(x1, y1)) = (&adv[0], &adv[1]) {
            if x0 == x1 && y0 == y1 {
                died[0] = true;
                died[1] = true;
            }
        }

        // Crossing: snakes swap positions in one tick (pass through each other)
        if let (Some(p1h), Some(p2h)) = (self.snakes[0].head(), self.snakes[1].head()) {
            if let (Advance::Move(x0, y0), Advance::Move(x1, y1)) = (&adv[0], &adv[1]) {
                if (*x0, *y0) == p2h && (*x1, *y1) == p1h {
                    died[0] = true;
                    died[1] = true;
                }
            }
        }

        // Board collisions
        for pi in 0..2 {
            if died[pi] { continue; }
            match adv[pi] {
                Advance::Still => {}
                Advance::Crash => { died[pi] = true; }
                Advance::Move(nx, ny) => {
                    match self.board[ny][nx] {
                        Cell::Wall | Cell::P1Body | Cell::P2Body => { died[pi] = true; }
                        // Head cell is only lethal when the opponent is NOT moving away.
                        // If the opponent moves this tick, their head vacates the cell;
                        // the crossing check above handles the swap case.
                        Cell::P1Head => {
                            if pi != 0 && !matches!(adv[0], Advance::Move(..)) {
                                died[pi] = true;
                            }
                        }
                        Cell::P2Head => {
                            if pi != 1 && !matches!(adv[1], Advance::Move(..)) {
                                died[pi] = true;
                            }
                        }
                        Cell::P1Tail => {
                            // Tail retreats when its owner moves with no pending growth.
                            // Only collide if the tail actually stays this tick.
                            let stays = !matches!(adv[0], Advance::Move(..))
                                || self.snakes[0].grow_pending > 0;
                            if stays {
                                if pi == 0 { died[pi] = true; } else { attacked[pi] = true; }
                            }
                        }
                        Cell::P2Tail => {
                            let stays = !matches!(adv[1], Advance::Move(..))
                                || self.snakes[1].grow_pending > 0;
                            if stays {
                                if pi == 1 { died[pi] = true; } else { attacked[pi] = true; }
                            }
                        }
                        Cell::Food => { ate_food[pi] = true; }
                        Cell::Effect | Cell::Empty => {}
                    }
                }
            }
        }

        // Successful attack: attacker grows, opponent's tail tip is consumed.
        // Record the tail position so we can show a break-effect after redraw.
        let mut attack_pos: [Option<(usize, usize)>; 2] = [None; 2];
        for pi in 0..2 {
            if attacked[pi] && !died[pi] {
                self.snakes[pi].grow_pending += GROW_REWARD;
                let opp = 1 - pi;
                attack_pos[pi] = self.snakes[opp].body.back().copied();
                if self.snakes[opp].body.len() > 1 {
                    self.snakes[opp].body.pop_back();
                }
            }
        }

        // Tail attack outcome — winner resolved after animation.
        let won = [attacked[0] && !died[0], attacked[1] && !died[1]];

        // Move snakes (snake style: push new head, pop tail unless growing)
        for pi in 0..2 {
            if died[pi] { continue; }
            if let Advance::Move(nx, ny) = adv[pi] {
                let len = self.snakes[pi].body.len();
                let (mdx, mdy) = self.snakes[pi].dir;
                if self.snakes[pi].boost_ticks > 0 {
                    // Zero accum so there's no post-boost speed burst
                    self.snakes[pi].h_accum = 0;
                    self.snakes[pi].v_accum = 0;
                } else {
                    if mdx != 0 { self.snakes[pi].h_accum = self.snakes[pi].h_accum.saturating_sub(h_thresh(len)); }
                    if mdy != 0 { self.snakes[pi].v_accum = self.snakes[pi].v_accum.saturating_sub(v_thresh(len)); }
                }
                self.snakes[pi].body.push_front((nx, ny));
                let tail_for_afterburn: Option<(usize, usize)>;
                if self.snakes[pi].grow_pending > 0 {
                    self.snakes[pi].grow_pending -= 1; // skip tail pop → net +1 length
                    tail_for_afterburn = None;
                } else {
                    self.snakes[pi].move_count += 1;
                    if self.snakes[pi].move_count >= FOOD_DROP_INTERVAL {
                        self.snakes[pi].move_count = 0;
                        // Drop food at tail tip then always shrink by 1 → snake starves over time.
                        if let Some((tx, ty)) = self.snakes[pi].body.pop_back() {
                            self.board[ty][tx] = Cell::Food;
                        }
                        // Afterburn marks the shrink cell (the one that becomes empty space)
                        tail_for_afterburn = self.snakes[pi].body.back().copied();
                        self.snakes[pi].body.pop_back(); // unconditional shrink
                        if self.snakes[pi].body.is_empty() {
                            died[pi]           = true;
                            starved[pi]        = true;
                            starvation_head[pi] = Some((nx, ny));
                        }
                    } else {
                        tail_for_afterburn = self.snakes[pi].body.back().copied();
                        self.snakes[pi].body.pop_back(); // normal move, no food
                    }
                }
                if ate_food[pi] {
                    self.snakes[pi].grow_pending += FOOD_REWARD;
                }
                // Afterburn: spawn * at the cell the tail just vacated
                let pre_boost = self.snakes[pi].boost_ticks;
                if pre_boost > 0 {
                    if let Some((tx, ty)) = tail_for_afterburn {
                        self.boost_particles.push(Particle { x: tx, y: ty, ch: '*', life: 4 });
                    }
                    self.snakes[pi].boost_ticks -= 1;
                    // Extra horizontal half-step on alternate boost ticks → 1.5× rate.
                    // pre_boost 8,6,4,2 fire the extra step (4 of 8 ticks) = 12 moves total.
                    let (mdx, _) = self.snakes[pi].dir;
                    if mdx != 0 && pre_boost % 2 == 0 && !died[pi] {
                        if let Some((hx, hy)) = self.snakes[pi].head() {
                            let nx2 = hx as i32 + mdx;
                            if nx2 <= 0 || nx2 >= W as i32 - 1 {
                                died[pi] = true; // wall
                            } else {
                                let (nx2u, ny2u) = (nx2 as usize, hy);
                                let cell2 = self.board[ny2u][nx2u];
                                // Mirror the main collision logic for the extra step.
                                let crash = match cell2 {
                                    Cell::Wall | Cell::P1Body | Cell::P2Body => true,
                                    // Head is only lethal when the opponent is not moving away.
                                    Cell::P1Head => pi != 0 && !matches!(adv[0], Advance::Move(..)),
                                    Cell::P2Head => pi != 1 && !matches!(adv[1], Advance::Move(..)),
                                    // Tail is lethal when it stays (owner stationary or growing).
                                    Cell::P1Tail => !matches!(adv[0], Advance::Move(..))
                                        || self.snakes[0].grow_pending > 0,
                                    Cell::P2Tail => !matches!(adv[1], Advance::Move(..))
                                        || self.snakes[1].grow_pending > 0,
                                    _ => false,
                                };
                                if crash {
                                    died[pi] = true;
                                } else {
                                    if cell2 == Cell::Food {
                                        self.snakes[pi].grow_pending += FOOD_REWARD;
                                    }
                                    let tail_pos = self.snakes[pi].body.back().copied();
                                    self.snakes[pi].body.push_front((nx2u, ny2u));
                                    self.snakes[pi].body.pop_back();
                                    if let Some((tx, ty)) = tail_pos {
                                        self.boost_particles.push(Particle { x: tx, y: ty, ch: '*', life: 4 });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        for pi in 0..2 {
            if died[pi] { self.snakes[pi].alive = false; }
        }

        let has_death = died[0] || died[1] || won[0] || won[1];

        // Starvation: scan the board NOW (before draw_snakes clears it) to collect
        // the dying snake's visible cell positions for the wither animation.
        // Crash/bite victims use a sequential explosion queue instead.
        let mut wither_particles: Vec<Particle> = Vec::new();
        let mut dizzy_anim: Option<DizzyAnim> = None;
        let anim_segs: VecDeque<(usize, usize, Cell)> = if has_death {
            for pi in 0..2 {
                if !starved[pi] { continue; }
                for y in 1..H - 1 {
                    for x in 1..W - 1 {
                        let is_me = if pi == 0 {
                            matches!(self.board[y][x], Cell::P1Head | Cell::P1Body | Cell::P1Tail)
                        } else {
                            matches!(self.board[y][x], Cell::P2Head | Cell::P2Body | Cell::P2Tail)
                        };
                        if is_me {
                            wither_particles.push(Particle { x, y, ch: '~', life: 7 });
                        }
                    }
                }
                // Dizzy spawns at the position the head was moving into when it starved.
                if dizzy_anim.is_none() {
                    if let Some((hx, hy)) = starvation_head[pi] {
                        dizzy_anim = Some(DizzyAnim { hx, hy, step: 0 });
                    }
                }
            }

            let mut seqs: [Vec<(usize, usize, Cell)>; 2] = [Vec::new(), Vec::new()];
            for pi in 0..2 {
                if starved[pi] { continue; } // starvation handled above
                let crash_victim  = died[pi];
                let attack_victim = won[1 - pi];
                if !crash_victim && !attack_victim { continue; }
                let snake = &self.snakes[pi];
                let len = snake.body.len();
                let cells: Vec<_> = snake.body.iter().enumerate().map(|(i, &(x, y))| {
                    let cell = if i == 0 {
                        if pi == 0 { Cell::P1Head } else { Cell::P2Head }
                    } else if i == len - 1 {
                        if pi == 0 { Cell::P1Tail } else { Cell::P2Tail }
                    } else {
                        if pi == 0 { Cell::P1Body } else { Cell::P2Body }
                    };
                    (x, y, cell)
                }).collect();
                seqs[pi] = if attack_victim && !crash_victim {
                    cells.into_iter().rev().collect()
                } else {
                    cells
                };
            }
            let max_len = seqs[0].len().max(seqs[1].len());
            let mut out = VecDeque::new();
            for i in 0..max_len {
                if let Some(&s) = seqs[0].get(i) { out.push_back(s); }
                if let Some(&s) = seqs[1].get(i) { out.push_back(s); }
            }
            out
        } else {
            VecDeque::new()
        };

        let pending_winner = if won[0] || won[1] {
            match (won[0], won[1]) { (true, false) => 0i8, (false, true) => 1, _ => 2 }
        } else {
            match (self.snakes[0].alive, self.snakes[1].alive) {
                (true, false) => 0, (false, true) => 1, _ => 2
            }
        };

        // Age boost-afterburn particles
        for p in self.boost_particles.iter_mut() { p.life = p.life.saturating_sub(1); }
        self.boost_particles.retain(|p| p.life > 0);

        self.draw_snakes();

        // Place break-effect markers at attacked tail positions (after redraw so
        // they aren't cleared by clear_snakes).
        for pos in attack_pos.iter().flatten() {
            let (ex, ey) = *pos;
            if matches!(self.board[ey][ex], Cell::Empty) {
                self.board[ey][ex] = Cell::Effect;
                self.effects.push(((ex, ey), self.tick + EFFECT_TICKS));
            }
        }

        if has_death {
            self.pending_winner = pending_winner;
            // Re-paint dying snake cells on the board so the animation can clear them
            // segment by segment (crash victims were wiped by draw_snakes; attack victims
            // are still alive=true and were redrawn — re-painting is harmless for them).
            for &(sx, sy, cell) in &anim_segs {
                if sy < H && sx < W { self.board[sy][sx] = cell; }
            }
            self.death_anim = Some(DeathAnim { segments: anim_segs, particles: wither_particles, dizzy: dizzy_anim });
        }
    }

    fn advance_death_anim(&mut self) {
        let mut anim = self.death_anim.take().unwrap();

        // Age boost particles so they don't freeze during death anim
        for p in self.boost_particles.iter_mut() { p.life = p.life.saturating_sub(1); }
        self.boost_particles.retain(|p| p.life > 0);

        // Age existing particles and remove expired ones.
        for p in anim.particles.iter_mut() {
            p.life = p.life.saturating_sub(1);
        }
        anim.particles.retain(|p| p.life > 0);

        // Explode the next segment(s).
        let segs_per_step = if anim.segments.len() > 20 { 2 } else { 1 };
        for _ in 0..segs_per_step {
            let Some((sx, sy, _)) = anim.segments.pop_front() else { break; };

            // Clear the segment from the board.
            if sy < H && sx < W
                && matches!(self.board[sy][sx],
                    Cell::P1Head | Cell::P1Body | Cell::P1Tail
                    | Cell::P2Head | Cell::P2Body | Cell::P2Tail)
            {
                self.board[sy][sx] = Cell::Empty;
            }

            // Bright burst at the exploding segment.
            anim.particles.push(Particle { x: sx, y: sy, ch: '*', life: 4 });

            // Scatter sparks into adjacent empty cells.
            for &(ddx, ddy) in &DIRS {
                let px = sx as i32 + ddx;
                let py = sy as i32 + ddy;
                if px > 0 && py > 0 && px < W as i32 - 1 && py < H as i32 - 1 {
                    let (px, py) = (px as usize, py as usize);
                    if matches!(self.board[py][px], Cell::Empty | Cell::Food | Cell::Effect) {
                        anim.particles.push(Particle { x: px, y: py, ch: '+', life: 2 });
                    }
                }
            }
        }

        // Once explosion/wither clears, run the dizzy-and-collapse phase.
        if anim.segments.is_empty() && anim.particles.is_empty() {
            if let Some(ref mut dizzy) = anim.dizzy {
                if dizzy.step == 0 { dizzy.step = 1; }
                let step = dizzy.step;
                let (hx, hy) = (dizzy.hx, dizzy.hy);
                Game::spawn_dizzy(&mut anim.particles, hx, hy, step);
                dizzy.step += 1;
            }
            if anim.dizzy.as_ref().map(|d| d.step > 1).unwrap_or(false) {
                anim.dizzy = None;
            }
        }

        if anim.segments.is_empty() && anim.particles.is_empty() && anim.dizzy.is_none() {
            self.over = true;
            self.winner = self.pending_winner;
        } else {
            self.death_anim = Some(anim);
        }
    }

    fn spawn_dizzy(particles: &mut Vec<Particle>, hx: usize, hy: usize, step: u32) {
        if step != 1 { return; }
        let hxi = hx as i32;
        let hyi = hy as i32;
        macro_rules! add {
            ($ox:expr, $oy:expr, $ch:expr, $life:expr) => {
                particles.push(Particle {
                    x: (hxi + $ox).clamp(1, W as i32 - 2) as usize,
                    y: (hyi + $oy).clamp(1, H as i32 - 2) as usize,
                    ch: $ch,
                    life: $life,
                });
            }
        }
        // Pop-and-punk burst: bright center + 8-point ring + outer sparks
        add!(0, 0, '*', 6);
        for &(ox, oy) in &[(2i32,0i32),(2,-1),(0,-1),(-2,-1),(-2,0),(-2,1),(0,1),(2,1)] {
            add!(ox, oy, '+', 4);
        }
        add!(0, -2, '.', 2); add!(0, 2, '.', 2);
        add!(4, 0, '.', 2); add!(-4, 0, '.', 2);
    }

    // Accumulator-based speed: each tick adds SPEED_INC to h_accum/v_accum.
    // A move fires when the accumulator reaches h_thresh(len) / v_thresh(len).
    // Shorter snakes have lower thresholds → fire more often → move faster.
    // v_thresh = 2 × h_thresh keeps vertical at half horizontal speed (terminal aspect).
    fn try_advance(&self, pi: usize) -> Advance {
        if !self.snakes[pi].alive { return Advance::Still; }
        let (dx, dy) = self.snakes[pi].dir;
        let len = self.snakes[pi].body.len();
        if self.snakes[pi].boost_ticks == 0 {
            if dx != 0 && self.snakes[pi].h_accum < h_thresh(len) { return Advance::Still; }
            if dy != 0 && self.snakes[pi].v_accum < v_thresh(len) { return Advance::Still; }
        }
        let Some((hx, hy)) = self.snakes[pi].head() else { return Advance::Still; };
        let nx = hx as i32 + dx;
        let ny = hy as i32 + dy;
        if nx <= 0 || ny <= 0 || nx >= W as i32 - 1 || ny >= H as i32 - 1 {
            Advance::Crash
        } else {
            Advance::Move(nx as usize, ny as usize)
        }
    }

    fn set_dir(&mut self, player: usize, dir: (i32, i32)) {
        if player == 0 && self.snakes[0].alive && !self.over {
            self.snakes[0].next_dir = dir;
        }
    }

    fn activate_boost(&mut self, pi: usize) {
        if !self.snakes[pi].alive || self.snakes[pi].boost_ticks > 0
            || self.snakes[pi].body.len() < 3 || self.over
        {
            return;
        }
        for _ in 0..2 {
            if let Some((tx, ty)) = self.snakes[pi].body.pop_back() {
                self.board[ty][tx] = Cell::Empty;
            }
        }
        self.snakes[pi].boost_ticks = 8;
    }

    fn render_screen(&self) -> Vec<u8> {
        let mut rows: Vec<Vec<char>> = self.board.iter()
            .map(|row| row.iter().map(|&c| cell_char(c)).collect())
            .collect();

        // Overlay boost afterburn particles
        for p in &self.boost_particles {
            if p.y < H && p.x < W { rows[p.y][p.x] = p.glyph(); }
        }

        // Overlay death-animation particles on top of the board.
        if let Some(ref anim) = self.death_anim {
            for p in &anim.particles {
                if p.y < H && p.x < W { rows[p.y][p.x] = p.glyph(); }
            }
        }

        // Blink snakes nearing starvation; period shrinks as length drops → escalating urgency.
        // Blink warning: long visible / short hidden when tail is longer;
        // as length drops the hidden fraction grows → escalating urgency.
        if self.death_anim.is_none() {
            for snake in &self.snakes {
                if !snake.alive || snake.body.len() >= 10 { continue; }
                let (period, off) = match snake.body.len() {
                    1 => (2,  1), // 50% hidden (fast strobe)
                    2 => (3,  2), // 67% hidden
                    3 => (4,  2), // 50% hidden
                    4 => (6,  1), // 17% hidden
                    5 => (8,  1), // 12% hidden
                    6 => (10, 1), // 10% hidden
                    7 => (12, 1), //  8% hidden
                    8 => (14, 1), //  7% hidden
                    _ => (16, 1), //  6% hidden (len 9, barely a flicker)
                };
                if self.tick % period < off {
                    for &(x, y) in &snake.body {
                        if y < H && x < W { rows[y][x] = ' '; }
                    }
                }
            }
        }

        let p1len = self.snakes[0].body.len();
        let p2len = self.snakes[1].body.len();
        let p1_status = if self.snakes[0].alive {
            if self.snakes[0].boost_ticks > 0 { "[BOOST]".into() } else { String::new() }
        } else { "[DEAD]".into() };
        let p2_status = if self.snakes[1].alive { String::new() } else { "[DEAD]".into() };
        let left  = format!("P1:{}{} WASD", p1len, p1_status);
        let right = format!("CPU[{}] P2:{}{}", self.cpu_strategy.label(), p2len, p2_status);
        let title = "** SNAKE **";

        rows[0] = {
            let center = W / 2 - title.len() / 2;
            let mut buf = vec![' '; W];
            for (i, c) in left.chars().enumerate()  { if i < W { buf[i] = c; } }
            for (i, c) in title.chars().enumerate() { if center + i < W { buf[center + i] = c; } }
            let rs = W.saturating_sub(right.len());
            for (i, c) in right.chars().enumerate() { if rs + i < W { buf[rs + i] = c; } }
            buf
        };

        rows[H - 1] = {
            let s = "WASD/hjkl/SPC=boost  |  tail+5  |  food+2  |  len<10=STARVING  |  R=restart  Q";
            let mut buf = vec![' '; W];
            for (i, c) in s.chars().enumerate() { if i < W { buf[i] = c; } }
            buf
        };

        if self.over {
            let msg = match self.winner {
                0 => "*** YOU WIN! ***",
                1 => "*** CPU WINS! ***",
                _ => "***   DRAW!   ***",
            };
            let sub = "  Press R to restart  ";
            let box_w = msg.len().max(sub.len()) + 4;
            let bx = (W - box_w) / 2;
            let by = H / 2 - 2;

            let top:     Vec<char> = format!("+{:-<w$}+", "", w = box_w - 2).chars().collect();
            let mid_msg: Vec<char> = format!("| {:^w$} |", msg, w = box_w - 4).chars().collect();
            let mid_sub: Vec<char> = format!("| {:^w$} |", sub, w = box_w - 4).chars().collect();
            let bot:     Vec<char> = format!("+{:-<w$}+", "", w = box_w - 2).chars().collect();

            for (li, line) in [&top, &mid_msg, &mid_sub, &bot].iter().enumerate() {
                let ry = by + li;
                if ry < H {
                    for (ci, &ch) in line.iter().enumerate() {
                        let rx = bx + ci;
                        if rx < W { rows[ry][rx] = ch; }
                    }
                }
            }
        }

        let lines: Vec<String> = rows.iter().map(|r| r.iter().collect()).collect();
        lines.join("\n\r").into_bytes()
    }
}

lazy_static! {
    static ref GAME: Mutex<Game> = Mutex::new(Game::new());
}

#[wasm_bindgen]
pub enum Button {
    Up = 1,
    Right = 2,
    Down = 4,
    Left = 8,
    Boost = 16,
    Restart = 200,
}

#[wasm_bindgen]
pub fn get_screen() -> Vec<u8> {
    GAME.lock().unwrap().render_screen()
}

#[wasm_bindgen]
pub fn key(button: Button) {
    let mut game = GAME.lock().unwrap();
    match button {
        Button::Up      => game.set_dir(0, (0, -1)),
        Button::Right   => game.set_dir(0, (1,  0)),
        Button::Down    => game.set_dir(0, (0,  1)),
        Button::Left    => game.set_dir(0, (-1, 0)),
        Button::Boost   => game.activate_boost(0),
        Button::Restart => *game = Game::new(),
    }
}

#[wasm_bindgen]
pub fn game_loop() {
    GAME.lock().unwrap().step();
}

#[wasm_bindgen]
pub fn reset() {
    *GAME.lock().unwrap() = Game::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game() -> Game {
        let mut g = Game::new();
        g.ai_enabled = false; // deterministic tests: disable AI direction overrides
        g
    }

    #[test]
    fn initial_lengths() {
        let g = make_game();
        assert_eq!(g.snakes[0].body.len(), INIT_LEN);
        assert_eq!(g.snakes[1].body.len(), INIT_LEN);
    }

    #[test]
    fn length_shrinks_on_food_drop_interval() {
        let mut g = make_game();
        // INIT_LEN=15 → h_thresh=150, period=1.5 ticks/move.
        // Moves fire at ticks 1,2,4,5,7 (5 moves after 7 steps) then tick 8 (6th move → drop).
        for _ in 0..7 { g.step(); }
        if g.snakes[0].alive { assert_eq!(g.snakes[0].body.len(), INIT_LEN); }
        g.step(); // 6th move: food drops, snake shrinks
        if g.snakes[0].alive { assert_eq!(g.snakes[0].body.len(), INIT_LEN - 1); }
    }

    #[test]
    fn wall_kills_snake() {
        let mut g = make_game();
        // P1: head at x=78, one step from right wall (x=79). Body extends left.
        g.snakes[0].body.clear();
        for i in 0..INIT_LEN { g.snakes[0].body.push_back((78 - i, 5)); }
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);
        // P2: far-right area, head to the left so moving right won't hit own body.
        g.snakes[1].body.clear();
        for i in 0..INIT_LEN { g.snakes[1].body.push_back((16 - i, 20)); } // head=16, body left to 2
        g.snakes[1].dir = (1, 0); g.snakes[1].next_dir = (1, 0);
        g.draw_snakes();
        g.step();
        assert!(!g.snakes[0].alive, "P1 should die hitting right wall");
        assert!(g.snakes[1].alive, "P2 unaffected");
    }

    #[test]
    fn head_to_head_kills_both() {
        let mut g = make_game();
        // P1 head at (40,10) moving right. P2 head at (42,10) moving left.
        // Both advance to (41,10) simultaneously → head-to-head.
        g.snakes[0].body.clear();
        for i in 0..INIT_LEN { g.snakes[0].body.push_back((40 - i, 10)); }
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);

        g.snakes[1].body.clear();
        for i in 0..INIT_LEN { g.snakes[1].body.push_back((42 + i, 10)); }
        g.snakes[1].dir = (-1, 0); g.snakes[1].next_dir = (-1, 0);

        g.draw_snakes();
        g.step();

        assert!(!g.snakes[0].alive, "P1 should die in head-to-head");
        assert!(!g.snakes[1].alive, "P2 should die in head-to-head");
        // Winner is set after the death animation completes.
        for _ in 0..200 { if g.over { break; } g.step(); }
        assert_eq!(g.winner, 2, "should be a draw");
    }

    #[test]
    fn body_collision_kills() {
        let mut g = make_game();
        // P2: horizontal body at y=16, head at (19,16) moving right. Body x=5..19.
        g.snakes[1].body.clear();
        for i in 0..INIT_LEN { g.snakes[1].body.push_back((19 - i, 16)); }
        g.snakes[1].dir = (1, 0); g.snakes[1].next_dir = (1, 0);
        // P1: head at (12,15) moving down → lands on P2 body at (12,16)
        g.snakes[0].body.clear();
        for i in 0..INIT_LEN { g.snakes[0].body.push_back((12, 15 - i)); }
        g.snakes[0].dir = (0, 1); g.snakes[0].next_dir = (0, 1);
        g.draw_snakes();
        // prime timer so vertical move is allowed on first step (delay=1, need timer>=2)
        g.step();
        assert!(!g.snakes[0].alive, "P1 should die hitting P2 body");
        assert!(g.snakes[1].alive, "P2 should survive");
    }

    #[test]
    fn attack_grows_attacker_shrinks_victim() {
        let mut g = make_game();
        // P2: horizontal on row 16, head at (10,16) moving left, tail at (24,16).
        g.snakes[1].body.clear();
        g.snakes[1].body.push_back((10, 16));
        for i in 1..INIT_LEN { g.snakes[1].body.push_back((10 + i, 16)); }
        g.snakes[1].dir = (-1, 0); g.snakes[1].next_dir = (-1, 0);

        // P1: vertical column 24, head at (24,15) moving DOWN → lands on P2 tail (24,16).
        g.snakes[0].body.clear();
        for i in 0..INIT_LEN { g.snakes[0].body.push_back((24, 15 - i)); }
        g.snakes[0].dir = (0, 1); g.snakes[0].next_dir = (0, 1);

        // P2 must be stationary this tick so its tail stays in place.
        // h_accum=0 → fires at 100 < h_thresh(15)=150 → Still.
        g.snakes[1].h_accum = 0;

        g.draw_snakes();
        let p2_before = g.snakes[1].body.len();
        g.step();

        assert!(g.snakes[0].alive,  "P1 survives the attack");
        assert!(g.snakes[1].alive,  "P2 survives being attacked");
        assert!(g.snakes[0].grow_pending > 0, "P1 gains pending growth");
        assert_eq!(g.snakes[1].body.len(), p2_before - 1, "P2 shrinks by 1");
        // Game over and winner are set after the death animation completes.
        for _ in 0..200 { if g.over { break; } g.step(); }
        assert!(g.over,         "game ends on successful attack");
        assert_eq!(g.winner, 0, "P1 wins by attack");
    }

    #[test]
    fn own_tail_is_lethal() {
        let mut g = make_game();
        // P1: head at (20,10) moving right, tail at (6,10). Arrange a U-shape
        // so the next move puts head at a cell occupied by own body.
        // Simplest: place P1 head one step before its own body segment.
        // P1 head (20,10), body goes up then right creating an L — but simpler:
        // just place the body so that (21,10) = P1Body.
        g.snakes[1].body.clear();
        for i in 0..INIT_LEN { g.snakes[1].body.push_back((60 + i, 10)); }
        g.snakes[1].dir = (1, 0); g.snakes[1].next_dir = (1, 0);

        g.snakes[0].body.clear();
        // Head at (20,10), body loops: goes left, then a segment at (21,10)
        // Arrange: (20,10) head, then (19..10,10), then (10,11), then (11..21,11), then (21,10)
        // That's 11 + 1 + 11 + 1 = 24 which exceeds INIT_LEN.
        // Simpler: just create a body where (21,10) is in it, head at (20,10).
        // Body: [20,10], [21,10], [21,11], [20,11], [19,11], [18,11],... going left
        let positions: Vec<(usize, usize)> = vec![
            (20,10),(21,10),(21,11),(20,11),(19,11),(18,11),(17,11),(16,11),
            (15,11),(14,11),(13,11),(12,11),(11,11),(10,11),(9,11),
        ];
        assert_eq!(positions.len(), INIT_LEN);
        for p in positions { g.snakes[0].body.push_back(p); }
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);

        g.draw_snakes();
        g.step();

        assert!(!g.snakes[0].alive, "P1 should die entering its own body");
    }

    // ── New collision tests ──────────────────────────────────────────────────

    #[test]
    fn starving_snake_body_is_still_lethal() {
        // Blink effect is purely visual; a snake at length < 10 must still kill opponents.
        let mut g = make_game();
        g.clear_snakes();

        // P1: len=3 (blink range), stationary this tick
        g.snakes[0].body.clear();
        g.snakes[0].body.push_back((42, 5));
        g.snakes[0].body.push_back((41, 5));
        g.snakes[0].body.push_back((40, 5));
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);
        g.snakes[0].h_accum = 0; // not moving

        // P2: head at (41, 6), moving up → (41, 5) = P1 body
        g.snakes[1].body.clear();
        g.snakes[1].body.push_back((41, 6));
        for i in 1..10 { g.snakes[1].body.push_back((41, 6 + i)); }
        g.snakes[1].dir = (0, -1); g.snakes[1].next_dir = (0, -1);
        g.snakes[1].v_accum = v_thresh(10);

        g.draw_snakes();
        g.step();

        assert!(!g.snakes[1].alive, "P2 must die running into P1 body even when P1 is blinking");
        assert!(g.snakes[0].alive, "P1 survives");
    }

    #[test]
    fn tail_stays_when_growing() {
        // When grow_pending > 0 the tail does not retreat; the opponent must be blocked.
        let mut g = make_game();
        g.clear_snakes();

        // P2: 2-cell snake moving left, grow_pending=2 → tail at (31,12) stays this tick
        g.snakes[1].body.clear();
        g.snakes[1].body.push_back((30, 12));
        g.snakes[1].body.push_back((31, 12));
        g.snakes[1].dir = (-1, 0); g.snakes[1].next_dir = (-1, 0);
        g.snakes[1].h_accum = h_thresh(2);
        g.snakes[1].grow_pending = 2;

        // P1: head at (31, 13), moving up → (31, 12) = P2 tail that stays
        g.snakes[0].body.clear();
        for i in 0..5 { g.snakes[0].body.push_back((31, 13 + i)); }
        g.snakes[0].dir = (0, -1); g.snakes[0].next_dir = (0, -1);
        g.snakes[0].v_accum = v_thresh(5);

        g.draw_snakes();
        g.step();

        // P1 attacked successfully (tail was lethal due to grow_pending).
        // grow_pending is GROW_REWARD from attack minus 1 already consumed in the move loop.
        assert!(g.snakes[0].alive, "P1 should survive the tail attack");
        assert!(g.snakes[0].grow_pending > 0, "P1 should gain attack reward");
        assert!(g.death_anim.is_some(), "attack triggers end-game animation");
    }

    #[test]
    fn tail_retreats_when_not_growing() {
        // When grow_pending == 0 the tail retreats; the chasing snake must pass safely.
        let mut g = make_game();
        g.clear_snakes();

        // P2: 2-cell snake moving left, grow_pending=0 → tail at (31,12) retreats
        g.snakes[1].body.clear();
        g.snakes[1].body.push_back((30, 12));
        g.snakes[1].body.push_back((31, 12));
        g.snakes[1].dir = (-1, 0); g.snakes[1].next_dir = (-1, 0);
        g.snakes[1].h_accum = h_thresh(2);
        g.snakes[1].grow_pending = 0;

        // P1: head at (31, 13), moving up → (31, 12) = where P2 tail was (now empty)
        g.snakes[0].body.clear();
        for i in 0..5 { g.snakes[0].body.push_back((31, 13 + i)); }
        g.snakes[0].dir = (0, -1); g.snakes[0].next_dir = (0, -1);
        g.snakes[0].v_accum = v_thresh(5);

        g.draw_snakes();
        g.step();

        assert!(g.snakes[0].alive, "P1 should survive entering the vacated tail cell");
        assert_eq!(g.snakes[0].grow_pending, 0, "no attack reward — tail was not lethal");
        assert!(!g.over, "game continues");
    }

    #[test]
    fn boost_extra_step_body_collision() {
        // P1 boosting right; P2 is stationary with head 2 cells ahead.
        // The extra boost half-step must detect the collision and kill P1.
        let mut g = make_game();
        g.clear_snakes();

        // P1: head at (10,12), boost_ticks=8 (even → extra step fires on this tick)
        g.snakes[0].body.clear();
        for i in 0..10 { g.snakes[0].body.push_back((10 - i, 12)); }
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);
        g.snakes[0].h_accum = h_thresh(10);
        g.snakes[0].boost_ticks = 8;

        // P2: stationary this tick. Length 11 → h_thresh=110; h_accum=0 → 100 < 110 → Still.
        // Head at (12,12) — exactly where the extra step lands.
        g.snakes[1].body.clear();
        for i in 0..11 { g.snakes[1].body.push_back((12 + i, 12)); }
        g.snakes[1].dir = (1, 0); g.snakes[1].next_dir = (1, 0);
        g.snakes[1].h_accum = 0; // not moving this tick

        g.draw_snakes();
        g.step();

        assert!(!g.snakes[0].alive, "P1 should die on extra boost step into P2 head");
    }

    #[test]
    fn boost_extra_step_eats_food() {
        // P1 boosting right; a food cell is exactly at the extra-step target.
        // The food must be consumed and grow reward must be granted.
        let mut g = make_game();
        g.clear_snakes();

        g.snakes[0].body.clear();
        for i in 0..10 { g.snakes[0].body.push_back((10 - i, 12)); }
        g.snakes[0].dir = (1, 0); g.snakes[0].next_dir = (1, 0);
        g.snakes[0].h_accum = h_thresh(10);
        g.snakes[0].boost_ticks = 8; // even → extra step fires

        // P2: far away, no interference
        g.snakes[1].body.clear();
        for i in 0..10 { g.snakes[1].body.push_back((60 - i, 20)); }
        g.snakes[1].dir = (1, 0); g.snakes[1].next_dir = (1, 0);
        g.snakes[1].h_accum = 0;

        g.draw_snakes();
        g.board[12][12] = Cell::Food; // at (12,12) — extra step target

        g.step();

        assert!(g.snakes[0].alive);
        assert_eq!(g.snakes[0].grow_pending, FOOD_REWARD,
            "food at extra-step target must grant grow reward");
    }

    #[test]
    fn starvation_triggers_dizzy_anim_and_reaches_game_over() {
        let mut g = make_game();

        // P1: single-cell body, one tick from triggering food drop → starvation
        g.clear_snakes();
        g.snakes[0].body.clear();
        g.snakes[0].body.push_back((40, 12));
        g.snakes[0].dir = (1, 0);
        g.snakes[0].next_dir = (1, 0);
        g.snakes[0].move_count = FOOD_DROP_INTERVAL - 1;
        g.snakes[0].h_accum = h_thresh(1); // fires on next tick

        // P2: parked far away, won't interfere
        g.snakes[1].body.clear();
        for i in 0..INIT_LEN { g.snakes[1].body.push_back((60 - i, 20)); }
        g.snakes[1].dir = (1, 0);
        g.snakes[1].next_dir = (1, 0);
        g.snakes[1].h_accum = 0;

        g.draw_snakes();
        g.step(); // P1 moves, food drop fires, body empties → starved

        assert!(!g.snakes[0].alive, "P1 should have starved");
        assert!(g.death_anim.is_some(), "death_anim must be set after starvation");
        assert!(g.death_anim.as_ref().unwrap().dizzy.is_some(), "dizzy phase must be queued");

        // Run animation to completion — should terminate within a few hundred ticks
        for _ in 0..300 {
            if g.over { break; }
            g.step();
        }
        assert!(g.over, "game must reach over=true after starvation animation");
    }
}
