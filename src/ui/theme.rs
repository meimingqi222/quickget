//! Material 3 浅色设计系统。色值与 QuickCleaner 同源，保证两个工具放一起不跳戏。

use gpui::{rgb, Hsla};

pub const BG: u32 = 0xf9f9f9;
pub const CARD: u32 = 0xffffff;
pub const SURF_LOW: u32 = 0xf3f3f3;
pub const SURF: u32 = 0xeeeeee;
pub const SURF_HIGH: u32 = 0xe8e8e8;
pub const SURF_HIGHEST: u32 = 0xe2e2e2;
pub const TEXT: u32 = 0x1a1c1c;
pub const MUTED: u32 = 0x404752;
pub const OUTLINE: u32 = 0x717783;
pub const OUTLINE_VAR: u32 = 0xc0c7d4;
pub const PRIMARY: u32 = 0x005faa;
pub const PRIMARY_BRIGHT: u32 = 0x0078d4;
pub const PRIMARY_FIXED: u32 = 0xd3e3ff;
pub const ON_PRIMARY: u32 = 0xffffff;
pub const CAUTION: u32 = 0x974700;
pub const CAUTION_CONTAINER: u32 = 0xffdbc8;
pub const ERROR: u32 = 0xba1a1a;
pub const ERROR_CONTAINER: u32 = 0xffdad6;
pub const OK: u32 = 0x1b6b3a;
pub const OK_CONTAINER: u32 = 0xd4edda;

pub fn rgba(hex: u32, alpha: f32) -> Hsla {
    let mut c = Hsla::from(rgb(hex));
    c.a = alpha;
    c
}
