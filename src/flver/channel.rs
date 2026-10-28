//! The variable-length attribute lists of a vertex ([`Channel`]): a growable list that keeps its first few values in
//! the vertex itself, so that the vertices of a mesh (millions in a map) do not each own a heap block per list.
use std::ops::{Deref, DerefMut};

/// A list of `T` that holds up to `N` values inline and moves to the heap beyond that. It reads and writes like a
/// `Vec<T>` (it dereferences to a slice and has `push`, `pop`, `truncate`, `clear` and `extend`) and compares equal to
/// the same values in a `Vec`, an array or a slice; where the values are kept is not observable.
#[derive(Clone)]
pub struct Channel<T, const N: usize> {
    repr: Repr<T, N>,
}

#[derive(Clone)]
enum Repr<T, const N: usize> {
    /// `len` values at the front of `items`; the rest of `items` is `T::default()`.
    Inline {
        len: u8,
        items: [T; N],
    },
    Heap(Vec<T>),
}

impl<T: Copy + Default, const N: usize> Channel<T, N> {
    /// An empty list.
    pub fn new() -> Self {
        Self {
            repr: Repr::Inline {
                len: 0,
                items: [T::default(); N],
            },
        }
    }

    /// Appends `value`; the list moves to the heap when it is full.
    pub fn push(&mut self, value: T) {
        match &mut self.repr {
            Repr::Inline { len, items } => match items.get_mut(usize::from(*len)) {
                Some(slot) => {
                    *slot = value;
                    *len = len.saturating_add(1);
                }
                None => {
                    let mut grown = Vec::with_capacity(N.saturating_mul(2).max(4));
                    grown.extend_from_slice(items);
                    grown.push(value);
                    self.repr = Repr::Heap(grown);
                }
            },
            Repr::Heap(values) => values.push(value),
        }
    }

    /// Removes and returns the last value.
    pub fn pop(&mut self) -> Option<T> {
        match &mut self.repr {
            Repr::Inline { len, items } => {
                *len = len.checked_sub(1)?;
                items.get(usize::from(*len)).copied()
            }
            Repr::Heap(values) => values.pop(),
        }
    }

    /// Shortens the list to `len` values (no change if it is already shorter).
    pub fn truncate(&mut self, len: usize) {
        match &mut self.repr {
            Repr::Inline { len: have, .. } => {
                *have = u8::try_from(len).map_or(*have, |len| len.min(*have));
            }
            Repr::Heap(values) => values.truncate(len),
        }
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Whether the values are on the heap (they stay inline while there are at most `N`).
    pub fn spilled(&self) -> bool {
        matches!(self.repr, Repr::Heap(_))
    }

    /// The bytes the values take on the heap: none while they are inline (the list's own size, which a vertex counts
    /// with itself, holds them).
    pub fn heap_bytes(&self) -> usize {
        match &self.repr {
            Repr::Inline { .. } => 0,
            Repr::Heap(values) => values.len().saturating_mul(std::mem::size_of::<T>()),
        }
    }
}

impl<T: Copy + Default, const N: usize> Default for Channel<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Deref for Channel<T, N> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match &self.repr {
            Repr::Inline { len, items } => items.get(..usize::from(*len)).unwrap_or_default(),
            Repr::Heap(values) => values,
        }
    }
}

impl<T, const N: usize> DerefMut for Channel<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        match &mut self.repr {
            Repr::Inline { len, items } => items.get_mut(..usize::from(*len)).unwrap_or_default(),
            Repr::Heap(values) => values,
        }
    }
}

impl<T: std::fmt::Debug, const N: usize> std::fmt::Debug for Channel<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // as the slice: a list prints the same whichever way it is kept
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl<T: serde::Serialize, const N: usize> serde::Serialize for Channel<T, N> {
    /// As a sequence, the way a `Vec` of the same values is written.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(serializer)
    }
}

impl<T: PartialEq, const N: usize, const M: usize> PartialEq<Channel<T, M>> for Channel<T, N> {
    fn eq(&self, other: &Channel<T, M>) -> bool {
        **self == **other
    }
}
impl<T: PartialEq, const N: usize> PartialEq<Vec<T>> for Channel<T, N> {
    fn eq(&self, other: &Vec<T>) -> bool {
        **self == **other
    }
}
impl<T: PartialEq, const N: usize> PartialEq<[T]> for Channel<T, N> {
    fn eq(&self, other: &[T]) -> bool {
        **self == *other
    }
}
impl<T: PartialEq, const N: usize> PartialEq<&[T]> for Channel<T, N> {
    fn eq(&self, other: &&[T]) -> bool {
        **self == **other
    }
}
impl<T: PartialEq, const N: usize, const M: usize> PartialEq<[T; M]> for Channel<T, N> {
    fn eq(&self, other: &[T; M]) -> bool {
        **self == other[..]
    }
}

impl<T: Copy + Default, const N: usize> From<Vec<T>> for Channel<T, N> {
    fn from(values: Vec<T>) -> Self {
        values.into_iter().collect()
    }
}
impl<T: Copy + Default, const N: usize, const M: usize> From<[T; M]> for Channel<T, N> {
    fn from(values: [T; M]) -> Self {
        values.into_iter().collect()
    }
}
impl<T: Copy + Default, const N: usize> FromIterator<T> for Channel<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut channel = Self::new();
        channel.extend(values);
        channel
    }
}
impl<T: Copy + Default, const N: usize> Extend<T> for Channel<T, N> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, values: I) {
        for value in values {
            self.push(value);
        }
    }
}
impl<'a, T, const N: usize> IntoIterator for &'a Channel<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, T, const N: usize> IntoIterator for &'a mut Channel<T, N> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::Channel;

    #[test]
    fn it_reads_and_writes_like_a_vec_whether_inline_or_on_the_heap() {
        let mut c: Channel<[f32; 2], 2> = Channel::new();
        let mut v: Vec<[f32; 2]> = Vec::new();
        assert!(c.is_empty() && c == v && !c.spilled());
        for i in 0..5u8 {
            let value = [f32::from(i), f32::from(i) * 2.];
            c.push(value);
            v.push(value);
            assert_eq!(c, v);
            assert_eq!(c.spilled(), v.len() > 2, "{} values", v.len());
            assert_eq!(format!("{c:?}"), format!("{v:?}"));
        }
        assert_eq!(c, v[..]);
        c[1] = [9., 9.];
        v[1] = [9., 9.];
        for x in &mut c {
            x[0] += 1.;
        }
        for x in &mut v {
            x[0] += 1.;
        }
        assert_eq!(c, v);
        assert_eq!(c.pop(), v.pop());
        c.truncate(3);
        v.truncate(3);
        assert_eq!(c, v);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.pop(), None);
        // inline: pops and pushes keep the values in order
        let mut small: Channel<u8, 3> = Channel::from([1u8, 2]);
        assert_eq!(small.pop(), Some(2));
        small.push(7);
        small.push(8);
        assert_eq!(small, [1, 7, 8]);
        assert!(!small.spilled());
        small.push(9);
        assert_eq!(small, [1, 7, 8, 9]);
        assert!(small.spilled());
        assert_eq!(small.clone(), small);
    }

    #[test]
    fn it_serializes_as_the_same_sequence_a_vec_does() {
        let c: Channel<[u8; 2], 1> = Channel::from([[1, 2], [3, 4]]);
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            serde_json::to_string(&vec![[1, 2], [3, 4]]).unwrap()
        );
        assert_eq!(serde_json::to_string(&Channel::<u8, 2>::new()).unwrap(), "[]");
    }

    #[test]
    fn it_is_collected_and_converted_in_order() {
        let c: Channel<u8, 2> = (0..6).collect();
        assert_eq!(c, vec![0, 1, 2, 3, 4, 5]);
        let d: Channel<u8, 2> = Channel::from(vec![4, 5]);
        assert!(!d.spilled());
        assert_eq!(c.iter().sum::<u8>(), 15);
        let e: Channel<u8, 4> = Channel::from(vec![4, 5]);
        assert_eq!(d, e);
    }
}
