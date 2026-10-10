//! Little-endian binary (de)serialisation for SNG records.
//!
//! Every record implements [`Bin`]. Structs are declared with [`bin_struct!`],
//! whose field order *is* the on-disk layout, so reading and writing cannot
//! drift apart.

use crate::{Error, Result};

/// Bounds-checked reader over a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(Error::Truncated)?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}

/// A value with a fixed little-endian binary layout.
pub trait Bin: Sized {
    fn read(r: &mut Reader) -> Result<Self>;
    fn write(&self, w: &mut Vec<u8>);
}

macro_rules! bin_number {
    ($($t:ty),*) => {$(
        impl Bin for $t {
            fn read(r: &mut Reader) -> Result<Self> {
                Ok(<$t>::from_le_bytes(r.array()?))
            }
            fn write(&self, w: &mut Vec<u8>) {
                w.extend_from_slice(&self.to_le_bytes());
            }
        }
    )*};
}
bin_number!(i8, u8, i16, u16, i32, u32, f32, f64);

/// Length-prefixed (i32) array.
impl<T: Bin> Bin for Vec<T> {
    fn read(r: &mut Reader) -> Result<Self> {
        let len = i32::read(r)?;
        let len = usize::try_from(len).map_err(|_| Error::Corrupt("negative array length"))?;
        // every element takes at least one byte: never pre-allocate more than the input can hold
        let mut v = Vec::with_capacity(len.min(r.remaining()));
        for _ in 0..len {
            v.push(T::read(r)?);
        }
        Ok(v)
    }
    fn write(&self, w: &mut Vec<u8>) {
        (self.len() as i32).write(w);
        for x in self {
            x.write(w);
        }
    }
}

/// Fixed-size array without a length prefix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fixed<T, const N: usize>(pub [T; N]);

impl<T: Default + Copy, const N: usize> Default for Fixed<T, N> {
    fn default() -> Self {
        Fixed([T::default(); N])
    }
}

impl<T: Bin + Default + Copy, const N: usize> Bin for Fixed<T, N> {
    fn read(r: &mut Reader) -> Result<Self> {
        let mut out = [T::default(); N];
        for x in &mut out {
            *x = T::read(r)?;
        }
        Ok(Fixed(out))
    }
    fn write(&self, w: &mut Vec<u8>) {
        for x in &self.0 {
            x.write(w);
        }
    }
}

impl<T, const N: usize> std::ops::Deref for Fixed<T, N> {
    type Target = [T; N];
    fn deref(&self) -> &[T; N] {
        &self.0
    }
}

impl<T, const N: usize> std::ops::DerefMut for Fixed<T, N> {
    fn deref_mut(&mut self) -> &mut [T; N] {
        &mut self.0
    }
}

/// Zero-terminated UTF-8 string in a fixed-size field of `N` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FixedStr<const N: usize>(pub String);

impl<const N: usize> Bin for FixedStr<N> {
    fn read(r: &mut Reader) -> Result<Self> {
        let raw = r.take(N)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(N);
        Ok(FixedStr(String::from_utf8_lossy(&raw[..end]).into_owned()))
    }
    fn write(&self, w: &mut Vec<u8>) {
        // always leave room for the terminator; never split a UTF-8 sequence
        let mut len = self.0.len().min(N - 1);
        while !self.0.is_char_boundary(len) {
            len -= 1;
        }
        w.extend_from_slice(&self.0.as_bytes()[..len]);
        w.resize(w.len() + N - len, 0);
    }
}

impl<const N: usize> From<&str> for FixedStr<N> {
    fn from(s: &str) -> Self {
        FixedStr(s.to_string())
    }
}

impl<const N: usize> std::fmt::Display for FixedStr<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `N` bytes the game ignores: read and dropped, written as zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pad<const N: usize>;

impl<const N: usize> Bin for Pad<N> {
    fn read(r: &mut Reader) -> Result<Self> {
        r.take(N)?;
        Ok(Pad)
    }
    fn write(&self, w: &mut Vec<u8>) {
        w.resize(w.len() + N, 0);
    }
}

/// Declares a record whose field order is its binary layout.
macro_rules! bin_struct {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct $name { $($(#[$fm])* pub $field: $ty),* }

        impl $crate::bin::Bin for $name {
            fn read(r: &mut $crate::bin::Reader) -> $crate::Result<Self> {
                Ok(Self { $($field: $crate::bin::Bin::read(r)?),* })
            }
            fn write(&self, w: &mut Vec<u8>) {
                $($crate::bin::Bin::write(&self.$field, w);)*
            }
        }
    };
}
pub(crate) use bin_struct;

/// Implements [`Bin`] for a `bitflags` type stored as its raw bits.
macro_rules! bin_flags {
    ($($t:ty),*) => {$(
        impl $crate::bin::Bin for $t {
            fn read(r: &mut $crate::bin::Reader) -> $crate::Result<Self> {
                Ok(<$t>::from_bits_retain($crate::bin::Bin::read(r)?))
            }
            fn write(&self, w: &mut Vec<u8>) {
                $crate::bin::Bin::write(&self.bits(), w);
            }
        }
    )*};
}
pub(crate) use bin_flags;
