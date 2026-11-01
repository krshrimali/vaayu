//! The size of whatever display the editor is painting on. A terminal is asked
//! directly; the web frontend reports its grid and cell pixels here instead,
//! because the controlling terminal (if any) has nothing to do with the tab.
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

const fn pack(a: u16, b: u16) -> u32 {
    ((a as u32) << 16) | b as u32
}

fn unpack(value: u32) -> (u16, u16) {
    ((value >> 16) as u16, value as u16)
}

static GRID: AtomicU32 = AtomicU32::new(0);
static CELL: AtomicU32 = AtomicU32::new(0);

pub fn set_grid(cols: u16, rows: u16) {
    GRID.store(pack(cols, rows), Ordering::Relaxed);
}

pub fn set_cell_pixels(width: u16, height: u16) {
    CELL.store(pack(width, height), Ordering::Relaxed);
}

pub fn size() -> io::Result<(u16, u16)> {
    match GRID.load(Ordering::Relaxed) {
        0 => crossterm::terminal::size(),
        grid => Ok(unpack(grid)),
    }
}

pub fn cell_pixels() -> Option<(u16, u16)> {
    match CELL.load(Ordering::Relaxed) {
        0 => None,
        cell => Some(unpack(cell)),
    }
}
