use std::{collections::HashSet, rc::Rc};

use super::task::TasksByStatus;

#[derive(Debug, Clone)]
pub struct SearchResults {
    query: String,
    // We use Rc<str> instead of String here for two reasons:
    // - Rc for cheap clones since elements in `matches` will always be in `tasks` as well
    // - Rc<str> implements Borrow<str> meaning we can query a `HashSet<Rc<str>>` using a `&str`
    // We do not modify the provided task names so we do not need the capabilities of String.
    tasks: Vec<Rc<str>>,
    matches: HashSet<Rc<str>>,
}

impl SearchResults {
    pub fn new(tasks: &TasksByStatus) -> Self {
        Self {
            tasks: tasks
                .task_names_in_displayed_order()
                .map(Rc::from)
                .collect(),
            query: String::new(),
            matches: HashSet::new(),
        }
    }

    /// Updates search results with new search body
    pub fn update_tasks(&mut self, tasks: &TasksByStatus) {
        self.tasks.clear();
        self.tasks
            .extend(tasks.task_names_in_displayed_order().map(Rc::from));
        self.update_matches();
    }

    /// Updates the query and the matches
    pub fn modify_query(&mut self, modification: impl FnOnce(&mut String)) {
        modification(&mut self.query);
        self.update_matches();
    }

    fn update_matches(&mut self) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        for task in self.tasks.iter().filter(|task| task.contains(&self.query)) {
            self.matches.insert(task.clone());
        }
    }

    /// Given an iterator it returns the first task that is in the search
    /// results
    pub fn first_match<'a>(&self, mut tasks: impl Iterator<Item = &'a str>) -> Option<&'a str> {
        tasks.find(|task| self.matches.contains(*task))
    }

    /// Returns if there are any matches for the query
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Returns query
    pub fn query(&self) -> &str {
        &self.query
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::tui::task::Task;

    fn basic_task_list() -> TasksByStatus {
        TasksByStatus {
            planned: vec![
                Task::new("app-a".into()),
                Task::new("app-b".into()),
                Task::new("pkg-a".into()),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_no_query_no_matches() {
        let task_list = basic_task_list();
        let results = SearchResults::new(&task_list);
        assert!(!results.has_matches());
    }

    #[test]
    fn test_matches_first_result() {
        let task_list = basic_task_list();
        let mut results = SearchResults::new(&task_list);
        results.modify_query(|s| s.push_str("app"));
        let result = results.first_match(task_list.task_names_in_displayed_order());
        assert_eq!(result, Some("app-a"));
        let result = results.first_match(task_list.task_names_in_displayed_order().skip(1));
        assert_eq!(result, Some("app-b"));
        let result = results.first_match(task_list.task_names_in_displayed_order().skip(2));
        assert_eq!(result, None);
    }

    #[test]
    fn test_update_task_rebuilds_matches() {
        let mut task_list = basic_task_list();
        let mut results = SearchResults::new(&task_list);
        results.modify_query(|s| s.push_str("app"));
        assert!(results.has_matches());
        task_list.planned.remove(0);
        task_list.planned.push(Task::new("app-c".into()));
        results.update_tasks(&task_list);
        assert!(results.has_matches());
        let result = results.first_match(task_list.task_names_in_displayed_order());
        assert_eq!(result, Some("app-b"));
        let result = results.first_match(task_list.task_names_in_displayed_order().skip(1));
        assert_eq!(result, Some("app-c"));
    }

    #[test]
    fn test_no_match_on_empty_list() {
        let task_list = basic_task_list();
        let mut results = SearchResults::new(&task_list);
        results.modify_query(|s| s.push_str("app"));
        assert!(results.has_matches());
        let result = results.first_match(std::iter::empty());
        assert_eq!(result, None);
    }
}
