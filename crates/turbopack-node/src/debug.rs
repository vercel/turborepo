use std::env;

const DEBUG_JS_VAR: &str = "TURBOPACK_DEBUG_JS";

pub fn should_debug(operation: &str) -> bool {
    let Ok(val) = env::var(DEBUG_JS_VAR) else {
		return false;
	};

    val == "*" || val.split(",").any(|part| part == operation)
}
