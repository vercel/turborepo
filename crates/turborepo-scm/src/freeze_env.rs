use std::{collections::HashMap, env};

pub struct FreezeEnv {
    frozen_vars: HashMap<String, String>,
}

impl FreezeEnv {
    /// 1. Captures the current environment variables.
    /// 1. Clears all environment variables.
    /// 1. Returns a binding that restores the environment variables when
    ///    dropped.
    pub fn capture() -> Self {
        // Capture all current environment variables
        let frozen_vars: HashMap<String, String> = env::vars().collect();

        // Clear all environment variables
        for key in frozen_vars.keys() {
            env::remove_var(key);
        }

        FreezeEnv { frozen_vars }
    }
}

impl Drop for FreezeEnv {
    /// Restores the saved environment variables when the guard is dropped.
    fn drop(&mut self) {
        // Restore all the saved environment variables
        for (key, value) in &self.frozen_vars {
            env::set_var(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn test_environment_guard() {
        let var = "MR_FREEZE";

        // Set an environment variable
        env::set_var(var, "123");

        {
            // Use the SealEnv to capture and clear all environment variables
            let _frozen_vars = FreezeEnv::capture();

            // Test that the environment variable is now cleared
            assert!(env::var(var).is_err());

            // Set a new environment variable just like the user would
            env::set_var(var, "456");

            // Test that the environment variable is now set
            assert_eq!(env::var(var).unwrap(), "456");
        }

        // After the guard is dropped, the environment variables should be restored
        assert_eq!(env::var(var).unwrap(), "123");
    }
}
