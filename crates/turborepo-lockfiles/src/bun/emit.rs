use regex::regex;

use super::{
    BunLockfile, PackageIdent, PackageInfo, PackageKey, is_git_or_github_package,
    is_tarball_or_url_package,
};

impl BunLockfile {
    pub(super) fn write_header(&self, output: &mut String) {
        output.push_str("{\n");
        output.push_str(&format!(
            "  \"lockfileVersion\": {},\n",
            self.data.lockfile_version
        ));
        // Write configVersion if present
        if let Some(config_version) = self.data.config_version {
            output.push_str(&format!("  \"configVersion\": {},\n", config_version));
        }
    }

    pub(super) fn write_workspaces(&self, output: &mut String) -> Result<(), crate::Error> {
        // serde_json uses 2-space indentation, but Bun uses 4-space
        output.push_str("  \"workspaces\": ");
        let workspaces_json = serde_json::to_string_pretty(&self.data.workspaces)?;

        let lines: Vec<&str> = workspaces_json.lines().collect();
        let mut adjusted_json = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted_json.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted_json.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }

        // Use the helper function to add trailing commas
        let workspaces_with_commas = Self::add_trailing_commas(&adjusted_json);

        output.push_str(&workspaces_with_commas);
        output.push_str(",\n");

        Ok(())
    }

    pub(super) fn write_trusted_dependencies(
        &self,
        output: &mut String,
    ) -> Result<(), crate::Error> {
        if self.data.trusted_dependencies.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.trusted_dependencies)?;
        output.push_str("  \"trustedDependencies\": ");
        output.push_str(&Self::format_json_field(&json));
        output.push_str(",\n");
        Ok(())
    }

    /// Add trailing commas to JSON values before closing brackets/braces
    /// Handles strings, numbers, booleans, nulls, and nested structures
    fn add_trailing_commas(json: &str) -> String {
        // Match: any JSON value (string, number, boolean, null, ] or }) followed by
        // newline+whitespace and then a closing bracket/brace
        // Pattern covers:
        // - Strings ending with "
        // - Numbers ending with digits
        // - Booleans: true, false
        // - Null: null
        // - Nested closings: ] or }
        let re = regex!(r#"("|true|false|null|\d|[\]}])\n(\s*)([\]}])"#);
        // Run multiple passes until no more changes (handles deeply nested structures)
        let mut result = json.to_string();
        loop {
            let new_result = re.replace_all(&result, "$1,\n$2$3").to_string();
            if new_result == result {
                break;
            }
            result = new_result;
        }
        result
    }

    /// Format a JSON value for Bun lockfile output with proper indentation and
    /// trailing commas.
    ///
    /// This helper consolidates the common formatting logic used by multiple
    /// write_* methods:
    /// 1. Adjusts indentation (adds 2 extra spaces to all lines after the
    ///    first)
    /// 2. Adds trailing commas (required by Bun's lockfile format)
    fn format_json_field(json: &str) -> String {
        let lines: Vec<&str> = json.lines().collect();
        let mut adjusted = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }
        Self::add_trailing_commas(&adjusted)
    }

    pub(super) fn write_overrides(&self, output: &mut String) -> Result<(), crate::Error> {
        if self.data.overrides.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.overrides)?;
        output.push_str("  \"overrides\": ");
        output.push_str(&Self::format_json_field(&json));
        output.push_str(",\n");
        Ok(())
    }

    pub(super) fn write_catalogs(&self, output: &mut String) -> Result<(), crate::Error> {
        // Write default catalog if present
        if !self.data.catalog.is_empty() {
            let json = serde_json::to_string_pretty(&self.data.catalog)?;
            output.push_str("  \"catalog\": ");
            output.push_str(&Self::format_json_field(&json));
            output.push_str(",\n");
        }

        // Write named catalogs if present
        if self.data.catalogs.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.catalogs)?;
        output.push_str("  \"catalogs\": ");
        output.push_str(&Self::format_json_field(&json));
        output.push_str(",\n");
        Ok(())
    }

    pub(super) fn write_packages(&self, output: &mut String) -> Result<(), crate::Error> {
        output.push_str("  \"packages\": {\n");

        let package_keys = self.sort_package_keys();
        for (i, key) in package_keys.iter().enumerate() {
            let entry = &self.data.packages[*key];

            let ident = PackageIdent::parse(&entry.ident);
            if ident.is_workspace() {
                // Workspace entries: [ident] when no info, [ident, info]
                // when there are dependencies. Bun omits the info object
                // entirely for workspace mappings with no dependencies.
                let ident_json = serde_json::to_string(&entry.ident)?;
                if let Some(info) = entry.info.as_ref().filter(|info| !info.is_empty()) {
                    let info_json = serde_json::to_string(info)?;
                    let info_json_spaced = self.format_info_json(&info_json);
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {info_json_spaced}],"
                    ));
                } else {
                    output.push_str(&format!("    \"{key}\": [{ident_json}],"));
                }
            } else if ident.is_local_package() || is_tarball_or_url_package(&entry.ident) {
                let ident_json = serde_json::to_string(&entry.ident)?;
                let info_json =
                    serde_json::to_string(&entry.info.as_ref().unwrap_or(&PackageInfo::default()))?;
                let info_json_spaced = self.format_info_json(&info_json);
                if let Some(checksum) = &entry.checksum {
                    if !checksum.is_empty() {
                        let checksum_json = serde_json::to_string(checksum)?;
                        output.push_str(&format!(
                            "    \"{key}\": [{ident_json}, {info_json_spaced}, {checksum_json}],",
                        ));
                    } else {
                        output.push_str(&format!(
                            "    \"{key}\": [{ident_json}, {info_json_spaced}],",
                        ));
                    }
                } else {
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {info_json_spaced}],",
                    ));
                }
            } else {
                let ident_json = serde_json::to_string(&entry.ident)?;
                let info_json =
                    serde_json::to_string(&entry.info.as_ref().unwrap_or(&PackageInfo::default()))?;
                let checksum_json = serde_json::to_string(entry.checksum.as_deref().unwrap_or(""))?;

                // Bun's format differs from serde_json: objects need padding spaces,
                // 3-element arrays get expanded with trailing commas, others stay compact
                let info_json_spaced = self.format_info_json(&info_json);

                // GitHub and git packages have 3 elements (no registry)
                // npm packages have 4 elements (with registry)
                if is_git_or_github_package(&entry.ident) {
                    // GitHub/git packages: [ident, info, checksum] - 3 elements
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {info_json_spaced}, {checksum_json}],",
                    ));
                } else {
                    // npm packages: [ident, registry, info, checksum] - 4 elements
                    let registry_json =
                        serde_json::to_string(entry.registry.as_deref().unwrap_or(""))?;
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {registry_json}, {info_json_spaced}, \
                         {checksum_json}],",
                    ));
                }
            }

            if i < package_keys.len() - 1 {
                output.push_str("\n\n");
            } else {
                output.push('\n');
            }
        }
        // Add comma if there are patched dependencies to follow
        if !self.data.patched_dependencies.is_empty() {
            output.push_str("  },\n");
        } else {
            output.push_str("  }\n");
        }

        Ok(())
    }

    pub(super) fn write_patched_dependencies(
        &self,
        output: &mut String,
    ) -> Result<(), crate::Error> {
        if self.data.patched_dependencies.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.patched_dependencies)?;
        output.push_str("  \"patchedDependencies\": ");
        output.push_str(&Self::format_json_field(&json));
        // No trailing comma - this is the last section before closing brace
        output.push('\n');
        Ok(())
    }

    /// Bun sorts packages by structure: regular packages, then scoped hoisted,
    /// then non-scoped hoisted, then deeply nested
    fn sort_package_keys(&self) -> Vec<&String> {
        // Sort priorities for package keys
        const SORT_PRIORITY_TOP_LEVEL: u8 = 1;
        const SORT_PRIORITY_SHALLOW_NESTED: u8 = 2;
        const SORT_PRIORITY_DEEP_NESTED: u8 = 3;
        const SORT_PRIORITY_VERY_DEEP_NESTED: u8 = 4;

        let mut package_keys: Vec<_> = self.data.packages.keys().collect();
        package_keys.sort_by(|a, b| {
            let category = |key_str: &str| -> u8 {
                let key = PackageKey::parse(key_str);
                match key {
                    PackageKey::Simple(_) => SORT_PRIORITY_TOP_LEVEL,
                    PackageKey::Scoped { .. } => SORT_PRIORITY_TOP_LEVEL,
                    PackageKey::Nested { .. } => SORT_PRIORITY_DEEP_NESTED,
                    PackageKey::ScopedNested { .. } => {
                        // Count slashes to determine nesting depth
                        let slash_count = key_str.matches('/').count();
                        if slash_count == 2 {
                            SORT_PRIORITY_SHALLOW_NESTED // @scope/parent/dep
                        } else {
                            SORT_PRIORITY_VERY_DEEP_NESTED // deeper nesting
                        }
                    }
                }
            };

            let a_cat = category(a);
            let b_cat = category(b);

            if a_cat != b_cat {
                a_cat.cmp(&b_cat)
            } else {
                a.cmp(b)
            }
        });
        package_keys
    }

    /// Formats JSON to match Bun's specific formatting requirements:
    /// - Objects need padding spaces: `{ "key": "value" }`
    /// - 3-element arrays get expanded with trailing commas: `[ item1, item2,
    ///   item3, ]`
    /// - Other arrays stay compact: `[item1, item2]`
    fn format_info_json(&self, info_json: &str) -> String {
        if info_json == "{}" {
            return info_json.to_string();
        }

        let mut result = String::with_capacity(info_json.len() + 100);
        let chars: Vec<char> = info_json.chars().collect();
        let mut i = 0;
        let mut in_string = false;
        let mut escape_next = false;

        while i < chars.len() {
            let c = chars[i];

            if !escape_next {
                if c == '"' {
                    in_string = !in_string;
                } else if c == '\\' && in_string {
                    escape_next = true;
                }
            } else {
                escape_next = false;
            }

            if !in_string {
                match c {
                    '{' => {
                        result.push_str("{ ");
                        i += 1;
                        continue;
                    }
                    '}' => {
                        result.push_str(" }");
                        i += 1;
                        continue;
                    }
                    ':' => {
                        result.push_str(": ");
                        i += 1;
                        continue;
                    }
                    '[' => {
                        let array_result = self.format_array(&chars, &mut i);
                        result.push_str(&array_result);
                        i += 1; // skip closing ]
                        continue;
                    }
                    ',' => {
                        result.push_str(", ");
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
            }

            result.push(c);
            i += 1;
        }

        result
    }

    /// Formats arrays according to Bun's requirements.
    /// Returns the formatted array string and updates the index.
    fn format_array(&self, chars: &[char], i: &mut usize) -> String {
        let mut array_depth = 1;
        let mut array_content = String::new();
        let mut in_array_string = false;
        let mut array_escape_next = false;
        *i += 1;

        while *i < chars.len() && array_depth > 0 {
            let array_char = chars[*i];

            if !array_escape_next {
                if array_char == '"' {
                    in_array_string = !in_array_string;
                } else if array_char == '\\' && in_array_string {
                    array_escape_next = true;
                } else if !in_array_string {
                    if array_char == '[' {
                        array_depth += 1;
                    } else if array_char == ']' {
                        array_depth -= 1;
                        if array_depth == 0 {
                            break;
                        }
                    }
                }
            } else {
                array_escape_next = false;
            }

            array_content.push(array_char);
            *i += 1;
        }

        let trimmed_content = array_content.trim_matches(|c: char| c == ',' || c.is_whitespace());

        // Bun uses compact arrays without trailing commas inside package entries
        format!("[{}]", self.format_array_content(trimmed_content))
    }

    /// Formats the content inside an array by adding proper spacing after
    /// commas.
    fn format_array_content(&self, content: &str) -> String {
        let mut formatted = String::with_capacity(content.len() + 20);
        let mut depth = 0;
        let mut in_str = false;
        let mut esc = false;

        for ch in content.chars() {
            if !esc {
                if ch == '"' {
                    in_str = !in_str;
                } else if ch == '\\' && in_str {
                    esc = true;
                } else if !in_str {
                    if ch == '[' || ch == '{' {
                        depth += 1;
                    } else if ch == ']' || ch == '}' {
                        depth -= 1;
                    } else if ch == ',' && depth == 0 {
                        formatted.push_str(", ");
                        continue;
                    }
                }
            } else {
                esc = false;
            }
            formatted.push(ch);
        }

        formatted
    }
}
