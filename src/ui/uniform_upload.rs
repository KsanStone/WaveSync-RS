use std::collections::HashMap;
use std::sync::Mutex;

/// Lives alongside the GPU buffers. Recreating resources also resets this cache.
#[derive(Default)]
pub(crate) struct UniformUploads(Mutex<HashMap<&'static str, Vec<u8>>>);

impl UniformUploads {
    pub(crate) fn if_changed(&self, name: &'static str, bytes: &[u8], upload: impl FnOnce(&[u8])) {
        let mut values = self.0.lock().unwrap();
        if values
            .get(name)
            .is_some_and(|previous| previous.as_slice() == bytes)
        {
            return;
        }
        upload(bytes);
        let previous = values.entry(name).or_default();
        previous.clear();
        previous.extend_from_slice(bytes);
    }
}

/// Compare the final uniform bytes and profile only writes that actually happen.
#[macro_export]
macro_rules! upload_uniform_if_changed {
    ($cache:expr, $name:literal, $queue:expr, $buffer:expr, $bytes:expr) => {
        $cache.if_changed($name, $bytes, |bytes| {
            $crate::profile_upload!(
                "upload.uniform",
                $name,
                $queue.write_buffer($buffer, 0, bytes)
            );
        })
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploads_on_first_use_and_change_but_not_when_unchanged() {
        let cache = UniformUploads::default();
        let mut writes = Vec::new();
        for data in [&[1, 2][..], &[1, 2][..], &[1, 3][..], &[1, 2][..]] {
            cache.if_changed("color", data, |bytes| writes.push(bytes.to_vec()));
        }
        assert_eq!(writes, vec![vec![1, 2], vec![1, 3], vec![1, 2]]);
    }
    #[test]
    fn buffers_and_recreated_resources_upload_independently() {
        let cache = UniformUploads::default();
        let mut count = 0;
        cache.if_changed("line", &[1], |_| count += 1);
        cache.if_changed("fill", &[1], |_| count += 1);
        cache.if_changed("line", &[1], |_| count += 1);
        UniformUploads::default().if_changed("line", &[1], |_| count += 1);
        assert_eq!(count, 3);
    }
}
