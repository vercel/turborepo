use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, OnceLock, RwLock},
    u8,
};

use console::{Style, StyledObject};

static COLORS: OnceLock<[Style; u8::MAX as usize]> = OnceLock::new();

pub fn get_terminal_package_colors() -> &'static [Style; u8::MAX as usize] {
    COLORS.get_or_init(|| {
        let colors: [Style; u8::MAX as usize] =
            core::array::from_fn(|index| Style::new().color256(index as u8));

        colors
    })
}

/// Selects colors for tasks and caches accordingly.
/// Shared between tasks so allows for concurrent access.
#[derive(Default)]
pub struct ColorSelector {
    inner: Arc<RwLock<ColorSelectorInner>>,
}

struct ColorSelectorInner {
    cache: HashMap<String, &'static Style>,
    colors_taken_state: [bool; u8::MAX as usize],
    total_colors_taken: u8,
}

impl Default for ColorSelectorInner {
    fn default() -> Self {
        Self {
            cache: Default::default(),
            colors_taken_state: [false; u8::MAX as usize],
            total_colors_taken: 0,
        }
    }
}

impl ColorSelector {
    pub fn color_for_key(&self, key: &str) -> &'static Style {
        if let Some(style) = self.inner.read().expect("lock poisoned").color(key) {
            return style;
        }

        let color = {
            self.inner
                .write()
                .expect("lock poisoned")
                .insert_color(key.to_string())
        };

        color
    }

    pub fn prefix_with_color(&self, cache_key: &str, prefix: &str) -> StyledObject<String> {
        if prefix.is_empty() {
            return Style::new().apply_to(String::new());
        }

        let style = self.color_for_key(cache_key);
        style.apply_to(format!("{}: ", prefix))
    }
}

impl ColorSelectorInner {
    fn color(&self, key: &str) -> Option<&'static Style> {
        self.cache.get(key).copied()
    }

    fn insert_color(&mut self, key: String) -> &'static Style {
        let colors = get_terminal_package_colors();
        let chosen_color = Self::get_color_id_by_key(self, &key);
        // A color might have been chosen by the time we get to inserting
        self.cache
            .entry(key)
            .or_insert_with(|| &colors[chosen_color])
    }

    pub fn get_color_hash_by_key(key: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn get_color_id_by_key(&mut self, key: &str) -> usize {
        let colors = get_terminal_package_colors();

        if self.total_colors_taken == u8::MAX {
            self.colors_taken_state = [false; u8::MAX as usize];
            self.total_colors_taken = 0;
        }

        let mut color_id: usize =
            (Self::get_color_hash_by_key(&key) % colors.len() as u64) as usize;

        let mut state: bool = *(self.colors_taken_state.get(color_id).unwrap());

        while state {
            color_id = (color_id + 1) % colors.len();
            state = *(self.colors_taken_state.get(color_id).unwrap());
        }

        self.total_colors_taken += 1;
        self.colors_taken_state[color_id] = true;
        return color_id;
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    #[test]
    fn test_color_selector() {
        let selector = super::ColorSelector::default();
        let color1 = selector.color_for_key("key1");
        let color2 = selector.color_for_key("key2");
        let color3 = selector.color_for_key("key1");
        assert_eq!(color1, color3);
        assert_ne!(color1, color2);
    }

    #[test]
    fn test_multithreaded_selector() {
        let selector = super::ColorSelector::default();
        thread::scope(|s| {
            s.spawn(|| {
                let color = selector.color_for_key("key1");
                assert_eq!(color, selector.color_for_key("key1"));
            });
            s.spawn(|| {
                let color = selector.color_for_key("key2");
                assert_eq!(color, selector.color_for_key("key2"));
            });
            s.spawn(|| {
                let color1 = selector.color_for_key("key1");
                let color2 = selector.color_for_key("key2");
                assert_eq!(color1, selector.color_for_key("key1"));
                assert_eq!(color2, selector.color_for_key("key2"));
                assert_ne!(color1, color2);
            });
        });
        // We only inserted 2 keys so next index should be 2
        assert_eq!(selector.inner.read().unwrap().total_colors_taken, 2);
    }

    #[test]
    fn test_rotation_after_all_colors_are_taken() {
        let selector = super::ColorSelector::default();

        let colors = super::get_terminal_package_colors();
        let num_colors = colors.len();

        // Exhaust all colors
        for i in 0..num_colors {
            let key = format!("package{}", i);
            selector.color_for_key(&key);
        }

        // At this point, all colors should be taken
        for state in selector
            .inner
            .read()
            .expect("lock poisoned")
            .colors_taken_state
            .iter()
            .take(num_colors)
        {
            assert_eq!(*state, true);
        }

        // The next key should start rotating from the beginning
        let key_next = format!("package{}", num_colors + 1);
        let next_color_id = selector.color_for_key(&key_next);

        // It should be the first color in the rotation again
        let next_key_color_id = (super::ColorSelectorInner::get_color_hash_by_key(&key_next)
            % colors.len() as u64) as usize;
        assert_eq!(next_color_id, &colors[next_key_color_id]);

        // At this point, all colors should be not taken, expect the one taken with the
        // latest package
        for (index, state) in selector
            .inner
            .read()
            .expect("lock poisoned")
            .colors_taken_state
            .iter()
            .enumerate()
        {
            if index == next_key_color_id {
                assert_eq!(*state, true);
            } else {
                assert_eq!(*state, false);
            }
        }

        assert_eq!(
            selector
                .inner
                .read()
                .expect("lock poisoned")
                .total_colors_taken,
            1
        );
    }
}
