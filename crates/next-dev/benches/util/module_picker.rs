use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};

/// Picks modules at random, but with a fixed seed so runs are somewhat
/// reproducible.
///
/// This must be initialized outside of `bench_with_input` so we don't repeat
/// the same sequence in different samples.
pub struct ModulePicker {
    depths: Vec<usize>,
    modules_by_depth: HashMap<usize, Vec<PathBuf>>,
    rng: parking_lot::Mutex<StdRng>,
}

impl ModulePicker {
    /// Creates a new module picker.
    pub fn new(modules: &[PathBuf], prefix: &Path) -> Self {
        let rng = StdRng::seed_from_u64(42);

        let mut modules_by_depth: HashMap<_, Vec<_>> = HashMap::new();
        for module in modules {
            let depth = module.strip_prefix(prefix).unwrap().components().count();

            modules_by_depth
                .entry(depth)
                .or_default()
                .push(module.clone());
        }
        let depths = modules_by_depth.keys().copied().collect();

        Self {
            depths,
            modules_by_depth,
            rng: parking_lot::Mutex::new(rng),
        }
    }

    /// Picks a random module with a uniform distribution over all depths.
    pub fn pick(&self) -> &PathBuf {
        let mut rng = self.rng.lock();
        // Sample from all depths uniformly.
        let depth = self.depths.choose(&mut *rng).unwrap();
        &self.modules_by_depth[depth].choose(&mut *rng).unwrap()
    }
}
