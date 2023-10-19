use std::fmt::{Debug, Display};

pub struct Type<T: Length>(usize, T);

impl<T: Default + Length> From<usize> for Type<T> {
    #[inline(always)]
    fn from(value: usize) -> Self {
        Self(value, Default::default())
    }
}

impl<T: Length> Debug for Type<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("index: {}, len: {}", self.0, self.1.len()))
    }
}

impl<T: Length> Display for Type<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("index: {}, len: {}", self.0, self.1.len()))
    }
}

pub struct FilledArray<const SIZE: usize>([u8; SIZE]);

impl<const SIZE: usize> Default for FilledArray<SIZE> {
    #[inline(always)]
    fn default() -> Self {
        Self([0; SIZE])
    }
}

pub struct FilledVec<const SIZE: usize>(Vec<u8>);

impl<const SIZE: usize> Default for FilledVec<SIZE> {
    #[inline(always)]
    fn default() -> Self {
        Self(vec![0; SIZE])
    }
}

pub trait Length {
    fn len(&self) -> usize;
}

impl<const SIZE: usize> Length for FilledArray<SIZE> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<const SIZE: usize> Length for FilledVec<SIZE> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub type StackType<const SIZE: usize> = Type<FilledArray<SIZE>>;
pub type HeapType<const SIZE: usize> = Type<FilledVec<SIZE>>;
