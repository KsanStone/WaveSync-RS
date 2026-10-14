//! Reuse allocations without confusing changing audio data with cached mappings.
pub(crate) struct CachedVec<K, T> {
    key: Option<K>,
    data: Vec<T>,
}
impl<K, T> Default for CachedVec<K, T> {
    fn default() -> Self {
        Self {
            key: None,
            data: Vec::new(),
        }
    }
}
impl<K: PartialEq, T> CachedVec<K, T> {
    /// Keep capacity, but force recomputation (and any corresponding GPU upload).
    pub fn invalidate(&mut self) {
        self.key = None;
    }
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }
    /// Returns true only when contents were rebuilt. Include every dependency
    /// in the key, or invalidate explicitly when an external dependency changes.
    pub fn update(&mut self, key: K, len: usize, mut element: impl FnMut(usize) -> T) -> bool {
        if self.key.as_ref() == Some(&key) && self.data.len() == len {
            return false;
        }
        self.data.clear();
        self.data.extend((0..len).map(&mut element));
        self.key = Some(key);
        true
    }
}

/// Refill changing data while retaining the allocation. Expressions run once.
#[macro_export]
macro_rules! refill_vec {
    ($vec:expr, $iter:expr) => {{
        let vec = &mut $vec;
        vec.clear();
        vec.extend($iter);
    }};
}

/// Ensure scratch length; existing elements are retained and must be overwritten
/// by the caller. Unlike vec![value; len], this reuses the existing allocation.
#[macro_export]
macro_rules! resize_scratch {
    ($vec:expr, $len:expr, $value:expr) => {{
        $vec.resize($len, $value);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_rebuilds_on_key_size_and_manual_invalidation_only() {
        let mut cache = CachedVec::default();
        assert!(cache.update(1, 4, |i| i));
        let ptr = cache.as_slice().as_ptr();
        assert!(!cache.update(1, 4, |_| panic!("unchanged key must not rebuild")));
        assert!(cache.update(2, 4, |i| i + 10));
        assert_eq!(cache.as_slice(), &[10, 11, 12, 13]);
        assert_eq!(cache.as_slice().as_ptr(), ptr);
        assert!(cache.update(2, 2, |i| i));
        cache.invalidate();
        assert!(cache.update(2, 2, |i| i + 20));
        assert_eq!(cache.as_slice(), &[20, 21]);
    }
    #[test]
    fn scratch_refills_and_resizes_keep_capacity_and_drop_stale_tail() {
        let mut vec = Vec::with_capacity(16);
        crate::refill_vec!(vec, 0..8);
        let ptr = vec.as_ptr();
        crate::refill_vec!(vec, 20..22);
        assert_eq!(vec, [20, 21]);
        crate::resize_scratch!(vec, 4, 0);
        assert_eq!(vec, [20, 21, 0, 0]);
        assert_eq!(vec.as_ptr(), ptr);
    }
}

/// Only the axis fields that change numeric mapping, excluding paint settings.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxisMappingKey {
    min_bits: u32,
    max_bits: u32,
    logarithmic: bool,
}
impl From<&crate::ui::plot::Axis> for AxisMappingKey {
    fn from(axis: &crate::ui::plot::Axis) -> Self {
        Self {
            min_bits: axis.min.to_bits(),
            max_bits: axis.max.to_bits(),
            logarithmic: axis.logarithmic,
        }
    }
}

#[cfg(test)]
mod axis_tests {
    use super::*;
    #[test]
    fn numeric_axis_changes_invalidate_mapping_but_highlights_do_not() {
        let mut axis = crate::ui::plot::Axis::linear(20.0, 20000.0);
        let mut cache = CachedVec::default();
        assert!(cache.update(AxisMappingKey::from(&axis), 2, |i| i));
        axis.highlight_values.push(440.0);
        assert!(!cache.update(AxisMappingKey::from(&axis), 2, |_| panic!(
            "paint-only change"
        )));
        axis.min = 40.0;
        assert!(cache.update(AxisMappingKey::from(&axis), 2, |i| i));
        axis.max = 10000.0;
        assert!(cache.update(AxisMappingKey::from(&axis), 2, |i| i));
        axis.logarithmic = true;
        assert!(cache.update(AxisMappingKey::from(&axis), 2, |i| i));
    }
}
