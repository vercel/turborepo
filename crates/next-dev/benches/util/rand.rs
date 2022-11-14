use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};

/// Picks `count` items from `vec` at random.
///
/// Calling this function with the same `count` and `vec` items will always
/// return the same items.
pub fn deterministic_random_pick<T>(mut vec: Vec<T>, count: usize) -> Vec<T>
where
    T: Ord,
{
    let mut rng = StdRng::seed_from_u64(42);
    vec.sort();
    vec.shuffle(&mut rng);
    vec.truncate(count);
    vec
}
