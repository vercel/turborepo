pub const TASK_DELIMITER: &str = "#";
pub const ROOT_PKG_NAME: &str = "//";

pub fn get_task_id(pkg_name: impl std::fmt::Display, target: &str) -> String {
    if is_package_task(target) {
        return target.to_owned();
    }
    format!("{}{}{}", pkg_name, TASK_DELIMITER, target)
}

pub fn root_task_id(target: &str) -> String {
    get_task_id(ROOT_PKG_NAME, target)
}

// TODO: Investigate if we should use split_once instead
pub fn get_package_task_from_id(task_id: &str) -> (String, String) {
    let arr: Vec<&str> = task_id.split(TASK_DELIMITER).collect();
    (arr[0].to_owned(), arr[1].to_owned())
}

pub fn root_task_task_name(task_id: &str) -> String {
    task_id
        .trim_start_matches(ROOT_PKG_NAME)
        .trim_start_matches(TASK_DELIMITER)
        .to_owned()
}

pub fn is_package_task(task: &str) -> bool {
    task.contains(TASK_DELIMITER) && !task.starts_with(TASK_DELIMITER)
}

pub fn is_task_in_package(task: &str, package_name: &str) -> bool {
    if !is_package_task(task) {
        return true;
    }
    let (package_name_expected, _) = get_package_task_from_id(task);
    package_name_expected == package_name
}

pub fn strip_package_name(task_id: &str) -> String {
    if is_package_task(task_id) {
        let (_, task) = get_package_task_from_id(task_id);
        task
    } else {
        task_id.to_string()
    }
}
