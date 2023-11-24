use chrono::{DateTime, Local};

type GroupPrefixFn = fn(group_name: &str, time: &DateTime<Local>) -> String;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorBehavior {
    pub group_prefix: GroupPrefixFn,
    pub group_suffix: GroupPrefixFn,
}

impl VendorBehavior {
    pub fn get_vendor_group_prefix(
        vendor_behavior: Option<&VendorBehavior>,
        task_prefix: &str,
        task_start_time: &DateTime<Local>,
    ) -> Option<String> {
        return vendor_behavior.and_then(|vendor_behavior| {
            let factory = vendor_behavior.group_prefix;

            Some(factory(task_prefix, task_start_time))
        });
    }

    pub fn get_vendor_group_suffix(
        vendor_behavior: Option<&VendorBehavior>,
        task_prefix: &str,
        task_finish_time: &DateTime<Local>,
    ) -> Option<String> {
        return vendor_behavior.and_then(|vendor_behavior| {
            let factory = vendor_behavior.group_suffix;

            Some(factory(task_prefix, task_finish_time))
        });
    }
}

#[cfg(test)]
mod tests {

    use chrono::{Local, TimeZone};
    use test_case::test_case;

    use crate::{vendors::get_vendors, Vendor, VendorBehavior};

    fn get_vendor(name: &str) -> Vendor {
        for v in get_vendors() {
            if v.name == name {
                return v.clone();
            }
        }

        unreachable!("vendor not found")
    }

    #[test_case("GitHub Actions", "::group::task-name", "::endgroup::"
    ; "github_actions_log_group")]
    #[test_case(
        "GitLab CI",
        "\\e[0Ksection_start:0:task-name\\r\\e[0Ktask-name",
        "\\e[0Ksection_end:0:task-name\\r\\e[0K"
        ;"GitLabCI_log_group"
    )]
    #[test_case(
        "TeamCity",
        "##teamcity[blockOpened name='task-name']",
        "##teamcity[blockClosed name='task-name']"
        ;"TeamCity_log_group"
    )]
    #[test_case(
        "Travis CI",
        "travis_fold:start:task-name\r\n",
        "travis_fold:end:task-name\r\n"
        ;"TravisCI_log_group"
    )]
    #[test_case("Azure Pipelines", "##[group]task-name\r\n", "##[endgroup]\r\n";"AzurePipelines_log_group")]
    fn test_vendor_behavior_log_groups(
        vendor_name: &str,
        expected_group_prefix: &str,
        expected_group_suffix: &str,
    ) {
        let github_vendor = get_vendor(vendor_name);
        let task_prefix = "task-name";

        let group_prefix = VendorBehavior::get_vendor_group_prefix(
            github_vendor.behavior.as_ref(),
            task_prefix,
            &Local.timestamp_nanos(0),
        );
        let group_suffix = VendorBehavior::get_vendor_group_suffix(
            github_vendor.behavior.as_ref(),
            task_prefix,
            &Local.timestamp_nanos(0),
        );

        assert_eq!(group_prefix, Some(String::from(expected_group_prefix)));
        assert_eq!(group_suffix, Some(String::from(expected_group_suffix)));
    }
}
