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

#[derive(Default)]
struct ColorSelectorInner {
    cache: HashMap<String, &'static Style>,
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
        let color_id = (Self::get_color_id_by_key(&key) % colors.len() as u64) as usize;
        let chosen_color = &colors[color_id];
        // A color might have been chosen by the time we get to inserting
        self.cache.entry(key).or_insert_with(|| chosen_color)
    }

    fn get_color_id_by_key(key: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
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
    }
}
