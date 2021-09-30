#[macro_use]
extern crate serde_derive;

use rand::Rng;
use tcod::colors::{self, Color};
use tcod::console::*;
use tcod::input::{self, Event, Key, Mouse};
use tcod::map::{FovAlgorithm, Map as FovMap};

use std::cmp;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};

// actual size of the window
const SCREEN_WIDTH: i32 = 100;
const SCREEN_HEIGHT: i32 = 60;

// size of the map
const MAP_WIDTH: i32 = 100;
const MAP_HEIGHT: i32 = 50;

// sizes and coordinates relevant for the GUI
const BAR_WIDTH: i32 = 20;
const BAR_TOP_PADDING: i32 = 1;
const BAR_LEFT_PADDING: i32 = 3;
const PANEL_HEIGHT: i32 = 8;
const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;
const MSG_X: i32 = BAR_WIDTH + 2;
const MSG_WIDTH: i32 = SCREEN_WIDTH - BAR_WIDTH - 2;
const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;
const INVENTORY_WIDTH: i32 = 50;
const CHARACTER_SCREEN_WIDTH: i32 = 30;

// parameters for house generator
const ROOM_MAX_SIZE: i32 = 10;
const ROOM_MIN_SIZE: i32 = 6;
const MAX_ROOMS: i32 = 30;

const FOV_ALGO: FovAlgorithm = FovAlgorithm::Basic; // default FOV algorithm
const FOV_LIGHT_WALLS: bool = true; // light walls or not
const TORCH_RADIUS: i32 = 10;

const LIMIT_FPS: i32 = 20; // 20 frames-per-second maximum

const COLOR_DARK_WALL: Color = Color { r: 0, g: 0, b: 100 };
const COLOR_LIGHT_WALL: Color = Color {
    r: 130,
    g: 110,
    b: 50,
};
const COLOR_DARK_GROUND: Color = Color {
    r: 50,
    g: 50,
    b: 150,
};
const COLOR_LIGHT_GROUND: Color = Color {
    r: 200,
    g: 180,
    b: 50,
};

// player will always be the first object
const PLAYER: usize = 0;

type Map = Vec<Vec<Tile>>;
type Messages = Vec<(String, Color)>;

/// A tile of the map and its properties
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Tile {
    blocked: bool,
    explored: bool,
    block_sight: bool,
}

impl Tile {
    pub fn empty() -> Self {
        Tile {
            blocked: false,
            explored: false,
            block_sight: false,
        }
    }

    pub fn wall() -> Self {
        Tile {
            blocked: true,
            explored: false,
            block_sight: true,
        }
    }
}

/// A rectangle on the map, used to characterise a room.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect {
            x1: x,
            y1: y,
            x2: x + w,
            y2: y + h,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        let center_x = (self.x1 + self.x2) / 2;
        let center_y = (self.y1 + self.y2) / 2;
        (center_x, center_y)
    }

    pub fn intersects_with(&self, other: &Rect) -> bool {
        // returns true if this rectangle intersects with another one
        (self.x1 <= other.x2)
            && (self.x2 >= other.x1)
            && (self.y1 <= other.y2)
            && (self.y2 >= other.y1)
    }
}

/// This is a generic object: the player, a monster, an item, the stairs...
/// It's always represented by a character on screen.
#[derive(Debug, Serialize, Deserialize)]
struct Object {
    x: i32,
    y: i32,
    char: char,
    color: Color,
    name: String,
    blocks: bool,
    alive: bool,
    stats: Option<Stats>,
    ai: Option<Ai>,
    item: Option<Item>,
    equipment: Option<Equipment>,
    always_visible: bool,
}

impl Object {
    pub fn new(x: i32, y: i32, char: char, name: &str, color: Color, blocks: bool) -> Self {
        Object {
            x: x,
            y: y,
            char: char,
            color: color,
            name: name.into(),
            blocks: blocks,
            alive: false,
            stats: None,
            ai: None,
            item: None,
            equipment: None,
            always_visible: false,
        }
    }

    /// set the color and then draw the character that represents this object at its position
    pub fn draw(&self, con: &mut Console) {
        con.set_default_foreground(self.color);
        con.put_char(self.x, self.y, self.char, BackgroundFlag::None);
    }

    /// Erase the character that represents this object
    pub fn clear(&self, con: &mut Console) {
        con.put_char(self.x, self.y, ' ', BackgroundFlag::None);
    }

    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn set_pos(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    /// return the distance to another object
    pub fn distance_to(&self, other: &Object) -> f32 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        ((dx.pow(2) + dy.pow(2)) as f32).sqrt()
    }

    /// return the distance to some coordinates
    pub fn distance(&self, x: i32, y: i32) -> f32 {
        (((x - self.x).pow(2) + (y - self.y).pow(2)) as f32).sqrt()
    }

    /// Equip object and show a message about it
    pub fn equip(&mut self, log: &mut Vec<(String, Color)>) {
        if self.item.is_none() {
            log.add(
                format!("Can't equip {:?} because it's not an Item.", self),
                colors::RED,
            );
            return;
        };
        if let Some(ref mut equipment) = self.equipment {
            if !equipment.equipped {
                equipment.equipped = true;
                log.add(
                    format!("Equipped {} on {}.", self.name, equipment.slot),
                    colors::LIGHT_GREEN,
                );
            }
        } else {
            log.add(
                format!("Can't equip {:?} because it's not an Equipment.", self),
                colors::RED,
            );
        }
    }

    /// Unequip object and show a message about it
    pub fn unequip(&mut self, log: &mut Vec<(String, Color)>) {
        if self.item.is_none() {
            log.add(
                format!("Can't unequip {:?} because it's not an Item.", self),
                colors::RED,
            );
            return;
        };
        if let Some(ref mut equipment) = self.equipment {
            if equipment.equipped {
                equipment.equipped = false;
                log.add(
                    format!("unequipped {} from {}.", self.name, equipment.slot),
                    colors::LIGHT_YELLOW,
                );
            }
        } else {
            log.add(
                format!("Can't unequip {:?} because it's not an Equipment.", self),
                colors::RED,
            );
        }
    }

    pub fn max_hunger(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_comfort(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_hygiene(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_bladder(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_energy(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_fun(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_social(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }

    pub fn max_room(&self) -> i32 {
        return self.stats.map_or(0, |s| s.base_max_all_stats);
    }
}

// character-related properties and methods (player, NPC).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Stats {
    base_max_all_stats: i32,
    hunger: i32,
    comfort: i32,
    hygiene: i32,
    bladder: i32,
    energy: i32,
    fun: i32,
    social: i32,
    room: i32,
    on_death: DeathCallback,
}

/// move by the given amount, if the destination is not blocked
fn move_by(id: usize, dx: i32, dy: i32, map: &Map, objects: &mut [Object]) {
    let (x, y) = objects[id].pos();
    if !is_blocked(x + dx, y + dy, map, objects) {
        objects[id].set_pos(x + dx, y + dy);
    }
}

/// add to the player's inventory and remove from the map
fn pick_item_up(object_id: usize, game: &mut Game) {
    if game.inventory.len() >= 26 {
        game.log.add(
            format!(
                "Your inventory is full, cannot pick up {}.",
                game.objects[object_id].name
            ),
            colors::RED,
        );
    } else {
        let item = game.objects.swap_remove(object_id);
        game.log
            .add(format!("You picked up a {}!", item.name), colors::GREEN);
        let index = game.inventory.len();
        let slot = item.equipment.map(|e| e.slot);
        game.inventory.push(item);

        // automatically equip, if the corresponding equipment slot is unused
        if let Some(slot) = slot {
            if get_equipped_in_slot(slot, &game.inventory).is_none() {
                game.inventory[index].equip(&mut game.log);
            }
        }
    }
}

fn get_equipped_in_slot(slot: Slot, inventory: &[Object]) -> Option<usize> {
    for (inventory_id, item) in inventory.iter().enumerate() {
        if item
            .equipment
            .as_ref()
            .map_or(false, |e| e.equipped && e.slot == slot)
        {
            return Some(inventory_id);
        }
    }
    None
}

fn is_blocked(x: i32, y: i32, map: &Map, objects: &[Object]) -> bool {
    // first test the map tile
    if map[x as usize][y as usize].blocked {
        return true;
    }
    // now check for any blocking objects
    objects
        .iter()
        .any(|object| object.blocks && object.x == x && object.y == y)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum DeathCallback {
    Player,
    NPC,
}

impl DeathCallback {
    fn callback(self, object: &mut Object, game: &mut Game) {
        let callback: fn(&mut Object, &mut Game) = match self {
            DeathCallback::Player => player_death,
            DeathCallback::NPC => npc_death,
        };
        callback(object, game);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Ai {
    Basic,
    Confused {
        previous_ai: Box<Ai>,
        num_turns: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Item {
    Heal,
    Lightning,
    Confuse,
    Fireball,
    Sword,
    Shield,
}

enum UseResult {
    UsedUp,
    UsedAndKept,
    Cancelled,
}

fn use_item(inventory_id: usize, game: &mut Game, tcod: &mut Tcod) {
    // just call the "use_function" if it is defined
    if let Some(item) = game.inventory[inventory_id].item {
        let on_use: fn(usize, &mut Game, &mut Tcod) -> UseResult = match item {
            Item::Heal => cast_heal,
            Item::Lightning => cast_lightning,
            Item::Confuse => cast_confuse,
            Item::Fireball => cast_fireball,
            Item::Sword => toggle_equipment,
            Item::Shield => toggle_equipment,
        };
        match on_use(inventory_id, game, tcod) {
            UseResult::UsedUp => {
                // destroy after use, unless it was cancelled for some reason
                game.inventory.remove(inventory_id);
            }
            UseResult::UsedAndKept => {} // do nothing
            UseResult::Cancelled => {
                game.log.add("Cancelled", colors::WHITE);
            }
        }
    } else {
        game.log.add(
            format!("The {} cannot be used.", game.inventory[inventory_id].name),
            colors::WHITE,
        );
    }
}

fn drop_item(inventory_id: usize, game: &mut Game) {
    let mut item = game.inventory.remove(inventory_id);
    if item.equipment.is_some() {
        item.unequip(&mut game.log);
    }
    item.set_pos(game.objects[PLAYER].x, game.objects[PLAYER].y);
    game.log
        .add(format!("You dropped a {}.", item.name), colors::YELLOW);
    game.objects.push(item);
}

/// return the position of a tile left-clicked in player's FOV (optionally in a
/// range), or (None,None) if right-clicked.
fn target_tile(
    tcod: &mut Tcod,
    objects: &[Object],
    game: &mut Game,
    max_range: Option<f32>,
) -> Option<(i32, i32)> {
    use tcod::input::KeyCode::Escape;
    loop {
        // render the screen. this erases the inventory and shows the names of
        // objects under the mouse.
        tcod.root.flush();
        let event = input::check_for_event(input::KEY_PRESS | input::MOUSE).map(|e| e.1);
        let mut key = None;
        match event {
            Some(Event::Mouse(m)) => tcod.mouse = m,
            Some(Event::Key(k)) => key = Some(k),
            None => {}
        }
        render_all(tcod, game, false);

        let (x, y) = (tcod.mouse.cx as i32, tcod.mouse.cy as i32);

        // accept the target if the player clicked in FOV, and in case a range
        // is specified, if it's in that range
        let in_fov = (x < MAP_WIDTH) && (y < MAP_HEIGHT) && tcod.fov.is_in_fov(x, y);
        let in_range = max_range.map_or(true, |range| objects[PLAYER].distance(x, y) <= range);
        if tcod.mouse.lbutton_pressed && in_fov && in_range {
            return Some((x, y));
        }

        let escape = key.map_or(false, |k| k.code == Escape);
        if tcod.mouse.rbutton_pressed || escape {
            return None; // cancel if the player right-clicked or pressed Escape
        }
    }
}

fn cast_heal(_inventory_id: usize, game: &mut Game, _tcod: &mut Tcod) -> UseResult {
    game.objects[PLAYER].stats.as_mut().unwrap().bladder += 20;

    UseResult::UsedUp
}

fn cast_lightning(_inventory_id: usize, game: &mut Game, tcod: &mut Tcod) -> UseResult {
    game.objects[PLAYER].stats.as_mut().unwrap().energy += 20;

    UseResult::UsedUp
}

fn cast_confuse(_inventory_id: usize, game: &mut Game, tcod: &mut Tcod) -> UseResult {
    game.objects[PLAYER].stats.as_mut().unwrap().social += 20;

    UseResult::UsedUp
}

fn cast_fireball(_inventory_id: usize, game: &mut Game, tcod: &mut Tcod) -> UseResult {
    game.objects[PLAYER].stats.as_mut().unwrap().comfort += 20;

    UseResult::UsedUp
}

fn toggle_equipment(inventory_id: usize, game: &mut Game, _tcod: &mut Tcod) -> UseResult {
    let equipment = match game.inventory[inventory_id].equipment {
        Some(equipment) => equipment,
        None => return UseResult::Cancelled,
    };
    if equipment.equipped {
        game.inventory[inventory_id].unequip(&mut game.log);
    } else {
        // if the slot is already being used, dequip whatever is there first
        if let Some(current) = get_equipped_in_slot(equipment.slot, &game.inventory) {
            game.inventory[current].unequip(&mut game.log);
        }
        game.inventory[inventory_id].equip(&mut game.log);
    }
    UseResult::UsedAndKept
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// An object that can be equipped, yielding bonuses.
struct Equipment {
    slot: Slot,
    equipped: bool,
    max_hp_bonus: i32,
    defense_bonus: i32,
    power_bonus: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Slot {
    LeftHand,
    RightHand,
    Head,
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Slot::LeftHand => write!(f, "left hand"),
            Slot::RightHand => write!(f, "right hand"),
            Slot::Head => write!(f, "head"),
        }
    }
}

fn create_room(room: Rect, map: &mut Map) {
    // go through the tiles in the rectangle and make them passable
    for x in (room.x1 + 1)..room.x2 {
        for y in (room.y1 + 1)..room.y2 {
            map[x as usize][y as usize] = Tile::empty();
        }
    }
}

fn create_h_tunnel(x1: i32, x2: i32, y: i32, map: &mut Map) {
    // horizontal tunnel. `min()` and `max()` are used in case `x1 > x2`
    for x in cmp::min(x1, x2)..(cmp::max(x1, x2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_v_tunnel(y1: i32, y2: i32, x: i32, map: &mut Map) {
    // vertical tunnel
    for y in cmp::min(y1, y2)..(cmp::max(y1, y2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn make_map(objects: &mut Vec<Object>, level: u32) -> Map {
    // fill map with "blocked" tiles
    let mut map = vec![vec![Tile::wall(); MAP_HEIGHT as usize]; MAP_WIDTH as usize];

    // Player is the first element, remove everything else.
    // NOTE: works only when the player is the first object!
    assert_eq!(&objects[PLAYER] as *const _, &objects[0] as *const _);
    objects.truncate(1);

    let mut rooms = vec![];

    for _ in 0..MAX_ROOMS {
        // random width and height
        let w = rand::thread_rng().gen_range(ROOM_MIN_SIZE..ROOM_MAX_SIZE + 1);
        let h = rand::thread_rng().gen_range(ROOM_MIN_SIZE..ROOM_MAX_SIZE + 1);
        // random position without going out of the boundaries of the map
        let x = rand::thread_rng().gen_range(0..MAP_WIDTH - w);
        let y = rand::thread_rng().gen_range(0..MAP_HEIGHT - h);

        let new_room = Rect::new(x, y, w, h);

        // run through the other rooms and see if they intersect with this one
        let failed = rooms
            .iter()
            .any(|other_room| new_room.intersects_with(other_room));

        if !failed {
            // this means there are no intersections, so this room is valid

            // "paint" it to the map's tiles
            create_room(new_room, &mut map);

            // add some content to this room
            place_objects(new_room, &map, objects, level);

            // center coordinates of the new room, will be useful later
            let (new_x, new_y) = new_room.center();

            if rooms.is_empty() {
                // this is the first room, where the player starts at
                objects[PLAYER].set_pos(new_x, new_y);
            } else {
                // all rooms after the first:
                // connect it to the previous room with a tunnel

                // center coordinates of the previous room
                let (prev_x, prev_y) = rooms[rooms.len() - 1].center();

                // toss a coin (random bool value -- either true or false)
                if rand::random() {
                    // first move horizontally, then vertically
                    create_h_tunnel(prev_x, new_x, prev_y, &mut map);
                    create_v_tunnel(prev_y, new_y, new_x, &mut map);
                } else {
                    // first move vertically, then horizontally
                    create_v_tunnel(prev_y, new_y, prev_x, &mut map);
                    create_h_tunnel(prev_x, new_x, new_y, &mut map);
                }
            }

            // finally, append the new room to the list
            rooms.push(new_room);
        }
    }

    // create stairs at the center of the last room
    let (last_room_x, last_room_y) = rooms[rooms.len() - 1].center();
    let mut stairs = Object::new(
        last_room_x,
        last_room_y,
        '<',
        "stairs",
        colors::WHITE,
        false,
    );
    stairs.always_visible = true;
    objects.push(stairs);

    map
}

struct Transition {
    level: u32,
    value: u32,
}

/// Returns a value that depends on level. the table specifies what
/// value occurs after each level, default is 0.
fn from_dungeon_level(table: &[Transition], level: u32) -> u32 {
    table
        .iter()
        .rev()
        .find(|transition| level >= transition.level)
        .map_or(0, |transition| transition.value)
}

fn place_objects(room: Rect, map: &Map, objects: &mut Vec<Object>, level: u32) {
    use rand::distributions::{Distribution, WeightedIndex};

    // maximum number of items per room
    let max_items = from_dungeon_level(
        &[
            Transition { level: 1, value: 1 },
            Transition { level: 4, value: 2 },
        ],
        level,
    );

    // item random table
    let item_chances = &mut [
        (Item::Heal, 35), // healing potion always shows up, even if all other items have 0 chance
        (
            Item::Lightning,
            from_dungeon_level(
                &[Transition {
                    level: 4,
                    value: 25,
                }],
                level,
            ),
        ),
        (
            Item::Fireball,
            from_dungeon_level(
                &[Transition {
                    level: 6,
                    value: 25,
                }],
                level,
            ),
        ),
        (
            Item::Confuse,
            from_dungeon_level(
                &[Transition {
                    level: 2,
                    value: 10,
                }],
                level,
            ),
        ),
        (
            Item::Sword,
            from_dungeon_level(&[Transition { level: 4, value: 5 }], level),
        ),
        (
            Item::Shield,
            from_dungeon_level(
                &[Transition {
                    level: 8,
                    value: 15,
                }],
                level,
            ),
        ),
    ];
    let item_choice = WeightedIndex::new(item_chances.iter().map(|item| item.1)).unwrap();

    // choose random number of items
    let num_items = rand::thread_rng().gen_range(0..max_items + 1);

    for _ in 0..num_items {
        // choose random spot for this item
        let x = rand::thread_rng().gen_range(room.x1 + 1..room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1..room.y2);

        // only place it if the tile is not blocked
        if !is_blocked(x, y, map, objects) {
            let mut item = match item_chances[item_choice.sample(&mut rand::thread_rng())].0 {
                Item::Heal => {
                    // create a healing potion
                    let mut object =
                        Object::new(x, y, '!', "healing potion", colors::VIOLET, false);
                    object.item = Some(Item::Heal);
                    object
                }
                Item::Lightning => {
                    // create a lightning bolt scroll
                    let mut object = Object::new(
                        x,
                        y,
                        '#',
                        "scroll of lightning bolt",
                        colors::LIGHT_YELLOW,
                        false,
                    );
                    object.item = Some(Item::Lightning);
                    object
                }
                Item::Fireball => {
                    // create a fireball scroll
                    let mut object =
                        Object::new(x, y, '#', "scroll of fireball", colors::LIGHT_YELLOW, false);
                    object.item = Some(Item::Fireball);
                    object
                }
                Item::Confuse => {
                    // create a confuse scroll
                    let mut object = Object::new(
                        x,
                        y,
                        '#',
                        "scroll of confusion",
                        colors::LIGHT_YELLOW,
                        false,
                    );
                    object.item = Some(Item::Confuse);
                    object
                }
                Item::Sword => {
                    // create a sword
                    let mut object = Object::new(x, y, '/', "sword", colors::SKY, false);
                    object.item = Some(Item::Sword);
                    object.equipment = Some(Equipment {
                        equipped: false,
                        slot: Slot::RightHand,
                        max_hp_bonus: 0,
                        defense_bonus: 0,
                        power_bonus: 3,
                    });
                    object
                }
                Item::Shield => {
                    // create a shield
                    let mut object = Object::new(x, y, '[', "shield", colors::DARKER_ORANGE, false);
                    object.item = Some(Item::Shield);
                    object.equipment = Some(Equipment {
                        equipped: false,
                        slot: Slot::LeftHand,
                        max_hp_bonus: 0,
                        defense_bonus: 1,
                        power_bonus: 0,
                    });
                    object
                }
            };
            item.always_visible = true;
            objects.push(item);
        }
    }
}

/// Advance to the next level
fn next_level(tcod: &mut Tcod, game: &mut Game) {
    game.log.add(
        "After a rare moment of peace, you descend deeper into \
         the heart of the dungeon...",
        colors::RED,
    );
    game.dungeon_level += 1;
    game.map = make_map(&mut game.objects, game.dungeon_level);
    initialise_fov(&game.map, tcod);
}

fn render_bar(
    panel: &mut Offscreen,
    x: i32,
    y: i32,
    total_width: i32,
    name: &str,
    value: i32,
    maximum: i32,
    bar_color: Color,
    back_color: Color,
) {
    // render a bar (HP, experience, etc). First calculate the width of the bar
    let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

    // render the background first
    panel.set_default_background(back_color);
    panel.rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

    // now render the bar on top
    panel.set_default_background(bar_color);
    if bar_width > 0 {
        panel.rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
    }

    // finally, some centered text with the values
    panel.set_default_foreground(colors::BLACK);
    panel.print_ex(
        x,
        y,
        BackgroundFlag::None,
        TextAlignment::Left,
        &format!("{: >7}: {}/{}", name, value, maximum),
    );
}

/// return a string with the names of all objects under the mouse
fn get_names_under_mouse(mouse: Mouse, objects: &[Object], fov_map: &FovMap) -> String {
    let (x, y) = (mouse.cx as i32, mouse.cy as i32);

    // create a list with the names of all objects at the mouse's coordinates and in FOV
    let names = objects
        .iter()
        .filter(|obj| obj.pos() == (x, y) && fov_map.is_in_fov(obj.x, obj.y))
        .map(|obj| obj.name.clone())
        .collect::<Vec<_>>();

    names.join(", ") // join the names, separated by commas
}

fn render_all(tcod: &mut Tcod, game: &mut Game, fov_recompute: bool) {
    if fov_recompute {
        // recompute FOV if needed (the player moved or something)
        let player = &game.objects[PLAYER];
        tcod.fov
            .compute_fov(player.x, player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO);

        // go through all tiles, and set their background color
        for y in 0..MAP_HEIGHT {
            for x in 0..MAP_WIDTH {
                let visible = tcod.fov.is_in_fov(x, y);
                let wall = game.map[x as usize][y as usize].block_sight;
                let color = match (visible, wall) {
                    // outside of field of view:
                    (false, true) => COLOR_DARK_WALL,
                    (false, false) => COLOR_DARK_GROUND,
                    // inside fov:
                    (true, true) => COLOR_LIGHT_WALL,
                    (true, false) => COLOR_LIGHT_GROUND,
                };

                let explored = &mut game.map[x as usize][y as usize].explored;
                if visible {
                    // since it's visible, explore it
                    *explored = true;
                }
                if *explored {
                    // show explored tiles only (any visible tile is explored already)
                    tcod.con
                        .set_char_background(x, y, color, BackgroundFlag::Set);
                }
            }
        }
    }

    let mut to_draw: Vec<_> = game
        .objects
        .iter()
        .filter(|o| {
            tcod.fov.is_in_fov(o.x, o.y)
                || (o.always_visible && game.map[o.x as usize][o.y as usize].explored)
        })
        .collect();
    // sort so that non-blocking objects come first
    to_draw.sort_by(|o1, o2| o1.blocks.cmp(&o2.blocks));
    // draw the objects in the list
    for object in &to_draw {
        object.draw(&mut tcod.con);
    }

    // blit the contents of "con" to the root console
    blit(
        &mut tcod.con,
        (0, 0),
        (MAP_WIDTH, MAP_HEIGHT),
        &mut tcod.root,
        (0, 0),
        1.0,
        1.0,
    );

    // prepare to render the GUI panel
    tcod.panel.set_default_background(colors::BLACK);
    tcod.panel.clear();

    // print the game messages, one line at a time
    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in game.log.iter().rev() {
        let msg_height = tcod.panel.get_height_rect(MSG_X, y, MSG_WIDTH, 0, msg);
        y -= msg_height;
        if y < 0 {
            break;
        }
        tcod.panel.set_default_foreground(color);
        tcod.panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg);
    }

    // show the player's stats
    let hunger = game.objects[PLAYER].stats.map_or(0, |p| p.hunger);
    let max_hunger = game.objects[PLAYER].max_hunger();
    let comfort = game.objects[PLAYER].stats.map_or(0, |p| p.comfort);
    let max_comfort = game.objects[PLAYER].max_comfort();
    let hygiene = game.objects[PLAYER].stats.map_or(0, |p| p.hygiene);
    let max_hygiene = game.objects[PLAYER].max_hygiene();
    let bladder = game.objects[PLAYER].stats.map_or(0, |p| p.bladder);
    let max_bladder = game.objects[PLAYER].max_bladder();
    let energy = game.objects[PLAYER].stats.map_or(0, |p| p.energy);
    let max_energy = game.objects[PLAYER].max_energy();
    let fun = game.objects[PLAYER].stats.map_or(0, |p| p.fun);
    let max_fun = game.objects[PLAYER].max_fun();
    let social = game.objects[PLAYER].stats.map_or(0, |p| p.social);
    let max_social = game.objects[PLAYER].max_social();
    let room = game.objects[PLAYER].stats.map_or(0, |p| p.room);
    let max_room = game.objects[PLAYER].max_room();

    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH * 2 - BAR_LEFT_PADDING,
        BAR_TOP_PADDING + 1,
        BAR_WIDTH,
        "Hunger",
        hunger,
        max_hunger,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH * 2 - BAR_LEFT_PADDING,
        BAR_TOP_PADDING + 2,
        BAR_WIDTH,
        "Comfort",
        comfort,
        max_comfort,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH * 2 - BAR_LEFT_PADDING,
        BAR_TOP_PADDING + 3,
        BAR_WIDTH,
        "Hygiene",
        hygiene,
        max_hygiene,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH * 2 - BAR_LEFT_PADDING,
        BAR_TOP_PADDING + 4,
        BAR_WIDTH,
        "Bladder",
        bladder,
        max_bladder,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH,
        BAR_TOP_PADDING + 1,
        BAR_WIDTH,
        "Energy",
        energy,
        max_energy,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH,
        BAR_TOP_PADDING + 2,
        BAR_WIDTH,
        "Fun",
        fun,
        max_fun,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH,
        BAR_TOP_PADDING + 3,
        BAR_WIDTH,
        "Social",
        social,
        max_social,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );
    render_bar(
        &mut tcod.panel,
        SCREEN_WIDTH - BAR_WIDTH,
        BAR_TOP_PADDING + 4,
        BAR_WIDTH,
        "Room",
        room,
        max_room,
        colors::LIGHT_GREEN,
        colors::DARKER_GREEN,
    );

    // display names of objects under the mouse
    tcod.panel.set_default_foreground(colors::LIGHT_GREY);
    tcod.panel.print_ex(
        1,
        0,
        BackgroundFlag::None,
        TextAlignment::Left,
        get_names_under_mouse(tcod.mouse, &mut game.objects, &tcod.fov),
    );

    // blit the contents of `panel` to the root console
    blit(
        &tcod.panel,
        (0, 0),
        (SCREEN_WIDTH, PANEL_HEIGHT),
        &mut tcod.root,
        (0, PANEL_Y),
        1.0,
        1.0,
    );
}

fn menu<T: AsRef<str>>(header: &str, options: &[T], width: i32, root: &mut Root) -> Option<usize> {
    assert!(
        options.len() <= 26,
        "Cannot have a menu with more than 26 options."
    );

    // calculate total height for the header (after auto-wrap) and one line per option
    let header_height = if header.is_empty() {
        0
    } else {
        root.get_height_rect(0, 0, width, SCREEN_HEIGHT, header)
    };
    let height = options.len() as i32 + header_height;

    // create an off-screen console that represents the menu's window
    let mut window = Offscreen::new(width, height);

    // print the header, with auto-wrap
    window.set_default_foreground(colors::WHITE);
    window.print_rect_ex(
        0,
        0,
        width,
        height,
        BackgroundFlag::None,
        TextAlignment::Left,
        header,
    );

    // print all the options
    for (index, option_text) in options.iter().enumerate() {
        let menu_letter = (b'a' + index as u8) as char;
        let text = format!("({}) {}", menu_letter, option_text.as_ref());
        window.print_ex(
            0,
            header_height + index as i32,
            BackgroundFlag::None,
            TextAlignment::Left,
            text,
        );
    }

    // blit the contents of "window" to the root console
    let x = SCREEN_WIDTH / 2 - width / 2;
    let y = SCREEN_HEIGHT / 2 - height / 2;
    tcod::console::blit(&mut window, (0, 0), (width, height), root, (x, y), 1.0, 0.7);

    // present the root console to the player and wait for a key-press
    root.flush();
    let key = root.wait_for_keypress(true);

    // convert the ASCII code to an index; if it corresponds to an option, return it
    if key.printable.is_alphabetic() {
        let index = key.printable.to_ascii_lowercase() as usize - 'a' as usize;
        if index < options.len() {
            Some(index)
        } else {
            None
        }
    } else {
        None
    }
}

fn inventory_menu(inventory: &[Object], header: &str, root: &mut Root) -> Option<usize> {
    // how a menu with each item of the inventory as an option
    let options = if inventory.len() == 0 {
        vec!["Inventory is empty.".into()]
    } else {
        inventory
            .iter()
            .map(|item| {
                // show additional information, in case it's equipped
                match item.equipment {
                    Some(equipment) if equipment.equipped => {
                        format!("{} (on {})", item.name, equipment.slot)
                    }
                    _ => item.name.clone(),
                }
            })
            .collect()
    };

    let inventory_index = menu(header, &options, INVENTORY_WIDTH, root);

    // if an item was chosen, return it
    if inventory.len() > 0 {
        inventory_index
    } else {
        None
    }
}

fn msgbox(text: &str, width: i32, root: &mut Root) {
    let options: &[&str] = &[];
    menu(text, options, width, root);
}

fn handle_keys(key: Key, tcod: &mut Tcod, game: &mut Game) -> PlayerAction {
    use tcod::input::KeyCode::*;

    let player_alive = game.objects[PLAYER].alive;
    match (key, player_alive) {
        (
            Key {
                code: Enter,
                alt: true,
                ..
            },
            _,
        ) => {
            // Alt+Enter: toggle fullscreen
            let fullscreen = tcod.root.is_fullscreen();
            tcod.root.set_fullscreen(!fullscreen);
            PlayerAction::DidntTakeTurn
        }
        (Key { code: Escape, .. }, _) => PlayerAction::Exit, // exit game

        // movement keys
        (Key { code: Up, .. }, true) | (Key { code: NumPad8, .. }, true) => {
            move_by(PLAYER, 0, -1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: Down, .. }, true) | (Key { code: NumPad2, .. }, true) => {
            move_by(PLAYER, 0, 1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: Left, .. }, true) | (Key { code: NumPad4, .. }, true) => {
            move_by(PLAYER, -1, 0, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: Right, .. }, true) | (Key { code: NumPad6, .. }, true) => {
            move_by(PLAYER, 1, 0, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: Home, .. }, true) | (Key { code: NumPad7, .. }, true) => {
            move_by(PLAYER, -1, -1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: PageUp, .. }, true) | (Key { code: NumPad9, .. }, true) => {
            move_by(PLAYER, 1, -1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: End, .. }, true) | (Key { code: NumPad1, .. }, true) => {
            move_by(PLAYER, -1, 1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: PageDown, .. }, true) | (Key { code: NumPad3, .. }, true) => {
            move_by(PLAYER, 1, 1, &game.map, &mut game.objects);
            PlayerAction::TookTurn
        }
        (Key { code: NumPad5, .. }, true) => {
            PlayerAction::TookTurn // do nothing, i.e. wait for the monster to come to you
        }

        (Key { printable: 'g', .. }, true) => {
            // pick up an item
            let item_id = game.objects.iter().position(|object| {
                object.pos() == game.objects[PLAYER].pos() && object.item.is_some()
            });
            if let Some(item_id) = item_id {
                pick_item_up(item_id, game);
            }
            PlayerAction::DidntTakeTurn
        }

        (Key { printable: 'i', .. }, true) => {
            // show the inventory: if an item is selected, use it
            let inventory_index = inventory_menu(
                &game.inventory,
                "Press the key next to an item to use it, or any other to cancel.\n",
                &mut tcod.root,
            );
            if let Some(inventory_index) = inventory_index {
                use_item(inventory_index, game, tcod);
            }
            PlayerAction::DidntTakeTurn
        }

        (Key { printable: 'd', .. }, true) => {
            // show the inventory; if an item is selected, drop it
            let inventory_index = inventory_menu(
                &game.inventory,
                "Press the key next to an item to drop it, or any other to cancel.\n'",
                &mut tcod.root,
            );
            if let Some(inventory_index) = inventory_index {
                drop_item(inventory_index, game);
            }
            PlayerAction::DidntTakeTurn
        }

        (Key { printable: '<', .. }, true) => {
            // go down stairs, if the player is on them
            let player_on_stairs = game.objects.iter().any(|object| {
                object.pos() == game.objects[PLAYER].pos() && object.name == "stairs"
            });
            if player_on_stairs {
                next_level(tcod, game);
            }
            PlayerAction::DidntTakeTurn
        }

        (Key { printable: 'c', .. }, true) => {
            // show character information
            let player = &game.objects[PLAYER];
            if let Some(stats) = player.stats.as_ref() {
                let msg = format!(
                    "Character information

Hunger: {}  Energy: {}
Comfort: {} Fun: {}
Hygiene: {} Social: {}
Bladder: {} Room: {}",
                    stats.hunger,
                    stats.energy,
                    stats.comfort,
                    stats.fun,
                    stats.hygiene,
                    stats.social,
                    stats.bladder,
                    stats.room
                );
                msgbox(&msg, CHARACTER_SCREEN_WIDTH, &mut tcod.root);
            }

            PlayerAction::DidntTakeTurn
        }

        _ => PlayerAction::DidntTakeTurn,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}

fn player_death(player: &mut Object, game: &mut Game) {
    // the game ended!
    game.log.add("You died!", colors::RED);

    // for added effect, transform the player into a corpse!
    player.char = '%';
    player.color = colors::DARK_RED;
}

fn npc_death(npc: &mut Object, game: &mut Game) {
    // transform it into a nasty corpse! it doesn't block, can't be
    // attacked and doesn't move
    game.log
        .add(format!("Oh no! {} is dead!", npc.name), colors::ORANGE);
    npc.char = '%';
    npc.color = colors::DARK_RED;
    npc.blocks = false;
    npc.stats = None;
    npc.ai = None;
    npc.name = format!("remains of {}", npc.name);
}

struct Tcod {
    root: Root,
    con: Offscreen,
    panel: Offscreen,
    fov: FovMap,
    mouse: Mouse,
}

#[derive(Serialize, Deserialize)]
struct Game {
    map: Map,
    log: Messages,
    inventory: Vec<Object>,
    dungeon_level: u32,
    objects: Vec<Object>,
}

trait MessageLog {
    fn add<T: Into<String>>(&mut self, message: T, color: Color);
}

impl MessageLog for Vec<(String, Color)> {
    fn add<T: Into<String>>(&mut self, message: T, color: Color) {
        self.push((message.into(), color));
    }
}

fn new_game(tcod: &mut Tcod) -> Game {
    // create object representing the player
    let mut player = Object::new(0, 0, '@', "player", colors::WHITE, true);
    player.alive = true;
    player.stats = Some(Stats {
        base_max_all_stats: 100,
        hunger: 100,
        comfort: 100,
        hygiene: 100,
        bladder: 100,
        energy: 100,
        fun: 100,
        social: 100,
        room: 100,
        on_death: DeathCallback::Player,
    });

    let mut objects = vec![player];
    let level = 1;

    let mut game = Game {
        // generate map (at this point it's not drawn to the screen)
        map: make_map(&mut objects, level),
        // create the list of game messages and their colors, starts empty
        log: vec![],
        inventory: vec![],
        dungeon_level: level,
        // the list of objects with just the player
        objects: objects,
    };

    // initial equipment: a dagger
    let mut dagger = Object::new(0, 0, '-', "dagger", colors::SKY, false);
    dagger.item = Some(Item::Sword);
    dagger.equipment = Some(Equipment {
        equipped: true,
        slot: Slot::LeftHand,
        max_hp_bonus: 0,
        defense_bonus: 0,
        power_bonus: 2,
    });
    game.inventory.push(dagger);

    initialise_fov(&game.map, tcod);

    // a warm welcoming message!
    game.log.add("Welcome to your new home!", colors::RED);

    game
}

fn initialise_fov(map: &Map, tcod: &mut Tcod) {
    // create the FOV map, according to the generated map
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            tcod.fov.set(
                x,
                y,
                !map[x as usize][y as usize].block_sight,
                !map[x as usize][y as usize].blocked,
            );
        }
    }

    // unexplored areas start black (which is the default background color)
    tcod.con.clear();
}

fn play_game(game: &mut Game, tcod: &mut Tcod) {
    // force FOV "recompute" first time through the game loop
    let mut previous_player_position = (-1, -1);

    let mut key = Default::default();

    while !tcod.root.window_closed() {
        match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
            Some((_, Event::Mouse(m))) => tcod.mouse = m,
            Some((_, Event::Key(k))) => key = k,
            _ => key = Default::default(),
        }

        // render the screen
        let fov_recompute = previous_player_position != (game.objects[PLAYER].pos());
        render_all(tcod, game, fov_recompute);

        tcod.root.flush();

        // erase all objects at their old locations, before they move
        for object in game.objects.iter_mut() {
            object.clear(&mut tcod.con)
        }

        // handle keys and exit game if needed
        previous_player_position = game.objects[PLAYER].pos();
        let player_action = handle_keys(key, tcod, game);
        if player_action == PlayerAction::Exit {
            save_game(game).unwrap();
            tcod.root.clear();
            tcod.root.flush();
            break;
        }
    }
}

fn save_game(game: &Game) -> Result<(), Box<Error>> {
    let save_data = serde_json::to_string(&game)?;
    let mut file = File::create("game.sav")?;
    file.write_all(save_data.as_bytes())?;
    Ok(())
}

fn load_game() -> Result<Game, Box<Error>> {
    let mut json_save_state = String::new();
    let mut file = File::open("game.sav")?;
    file.read_to_string(&mut json_save_state)?;
    let result = serde_json::from_str::<Game>(&json_save_state)?;
    Ok(result)
}

fn main_menu(tcod: &mut Tcod) {
    while !tcod.root.window_closed() {
        tcod.root.set_default_foreground(colors::LIGHT_YELLOW);
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT / 2 - 4,
            BackgroundFlag::None,
            TextAlignment::Center,
            "LARDUM",
        );
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT - 2,
            BackgroundFlag::None,
            TextAlignment::Center,
            "By Matt Fisher",
        );

        // show options and wait for the player's choice
        let choices = &["Play a new game", "Continue last game", "Quit"];
        let choice = menu("", choices, 24, &mut tcod.root);

        match choice {
            Some(0) => {
                // new game
                let mut game = new_game(tcod);
                play_game(&mut game, tcod);
            }
            Some(1) => {
                // load game
                match load_game() {
                    Ok(mut game) => {
                        initialise_fov(&game.map, tcod);
                        play_game(&mut game, tcod);
                    }
                    Err(_e) => {
                        msgbox("\nNo saved game to load.\n", 24, &mut tcod.root);
                        continue;
                    }
                }
            }
            Some(2) => {
                // quit
                break;
            }
            _ => {}
        }
    }
}

fn main() {
    let root = Root::initializer()
        .font("assets/consolas12x12_gs_tc.png", FontLayout::Tcod)
        .font_type(FontType::Greyscale)
        .size(SCREEN_WIDTH, SCREEN_HEIGHT)
        .title("Lardum")
        .init();
    tcod::system::set_fps(LIMIT_FPS);

    let mut tcod = Tcod {
        root: root,
        con: Offscreen::new(MAP_WIDTH, MAP_HEIGHT),
        panel: Offscreen::new(SCREEN_WIDTH, PANEL_HEIGHT),
        fov: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        mouse: Default::default(),
    };

    main_menu(&mut tcod);
}
