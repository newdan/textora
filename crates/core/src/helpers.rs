//! Random assortment of helpers I didn't know where to put.

use std::cmp::Ordering;
use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::{fmt, slice};

pub const KILO: usize = 1000;
pub const MEGA: usize = 1000 * 1000;
pub const GIGA: usize = 1000 * 1000 * 1000;

pub const KIBI: usize = 1024;
pub const MEBI: usize = 1024 * 1024;
pub const GIBI: usize = 1024 * 1024 * 1024;

pub struct MetricFormatter<T>(pub T);

impl fmt::Display for MetricFormatter<usize> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut value = self.0;
        let mut suffix = "B";
        if value >= GIGA {
            value /= GIGA;
            suffix = "GB";
        } else if value >= MEGA {
            value /= MEGA;
            suffix = "MB";
        } else if value >= KILO {
            value /= KILO;
            suffix = "kB";
        }
        write!(f, "{value}{suffix}")
    }
}

/// A viewport coordinate type used throughout the application.
pub type CoordType = isize;

/// To avoid overflow issues because you're adding two [`CoordType::MAX`]
/// values together, you can use [`COORD_TYPE_SAFE_MAX`] instead.
///
/// It equates to half the bits contained in [`CoordType`], which
/// for instance is 32767 (0x7FFF) when [`CoordType`] is a [`i32`].
pub const COORD_TYPE_SAFE_MAX: CoordType = (1 << (CoordType::BITS / 2 - 1)) - 1;

/// A 2D point. Uses [`CoordType`].
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Point {
    pub x: CoordType,
    pub y: CoordType,
}

impl Point {
    pub const MIN: Self = Self { x: CoordType::MIN, y: CoordType::MIN };
    pub const MAX: Self = Self { x: CoordType::MAX, y: CoordType::MAX };

    pub fn as_array(&mut self) -> &mut [CoordType; 2] {
        unsafe { &mut *(self as *mut Self as *mut [CoordType; 2]) }
    }
}

impl PartialOrd<Self> for Point {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Point {
    fn cmp(&self, other: &Self) -> Ordering {
        self.y.cmp(&other.y).then(self.x.cmp(&other.x))
    }
}

/// [`Read`] but with [`MaybeUninit<u8>`] buffers.
pub fn file_read_uninit<T: Read>(file: &mut T, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
    unsafe {
        let buf_slice = slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len());
        let n = file.read(buf_slice)?;
        Ok(n)
    }
}
