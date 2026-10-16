use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, Read},
};

use globwalk::PackageInput;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_fs::RepositoryReader;
use wax::{Glob, Program};

use crate::{Error, GitHashes, crlf};

struct InputPattern {
    glob: Glob<'static>,
    literal: Option<String>,
    prefix: AnchoredSystemPathBuf,
    ancestors: Vec<Glob<'static>>,
}

impl InputPattern {
    fn new(input: &PackageInput) -> Result<Self, Error> {
        let pattern = globwalk::fix_glob_pattern(input.as_str());
        let glob = Glob::new(pattern.as_ref())?.into_owned();
        let (prefix, _) = glob.clone().partition();
        let prefix = AnchoredSystemPathBuf::try_from(prefix.as_path())?;
        let literal = (input.is_literal() && glob.variance().is_invariant())
            .then(|| prefix.to_unix().to_string());
        let mut ancestors = Vec::new();
        for (index, _) in pattern.match_indices('/') {
            match Glob::new(&pattern[..index]) {
                Ok(ancestor) => ancestors.push(ancestor.into_owned()),
                // A group spanning components cannot be split here. Keep its
                // subtree reachable rather than interpreting glob syntax ourselves.
                Err(_) => {
                    let prefix = prefix.to_unix();
                    let subtree = if prefix.as_str().is_empty() {
                        "**".to_owned()
                    } else {
                        format!("{}/**", wax::escape(prefix.as_str()))
                    };
                    ancestors.push(Glob::new(&subtree)?.into_owned());
                    break;
                }
            }
        }
        Ok(Self {
            glob,
            literal,
            prefix,
            ancestors,
        })
    }

    fn reaches(&self, path: &str) -> bool {
        self.matches(path) || self.ancestors.iter().any(|glob| glob.is_match(path))
    }

    fn matches(&self, path: &str) -> bool {
        self.glob.is_match(path)
            || self.literal.as_ref().is_some_and(|prefix| {
                prefix.is_empty()
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
    }
}

/// Git's symlink blob describes the link, not the contents read by a task.
/// Replace those entries using logical input paths and confined, opened files.
pub(crate) fn extend<S: AsRef<str>>(
    root: &AbsoluteSystemPath,
    package: &AnchoredSystemPath,
    inputs: &[S],
    include_defaults: bool,
    attrs: (&AbsoluteSystemPath, Option<&crlf::GitAttrs>),
    indexed_links: Option<Vec<AbsoluteSystemPathBuf>>,
    hashes: &mut GitHashes,
) -> Result<(), Error> {
    if indexed_links.as_ref().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    let package_dir = root.resolve(package);
    let package_unix = package.to_unix();
    let defaults = inputs.is_empty() || include_defaults;
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut scopes = Vec::new();
    if defaults {
        scopes.push(package_dir.clone());
    }
    for raw in inputs {
        let input = PackageInput::resolve(package_unix.as_str(), raw.as_ref())?;
        if input.escapes_repo_root() {
            continue;
        }
        let compiled = InputPattern::new(&input)?;
        if input.is_exclusion() {
            excludes.push(compiled);
        } else {
            scopes.push(root.resolve(&compiled.prefix));
            includes.push(compiled);
        }
    }
    // Configs are implicit inputs even when authored globs don't select them.
    for config in ["package.json", "turbo.json", "turbo.jsonc"] {
        let input = PackageInput::resolve(package_unix.as_str(), config)?;
        let mut pattern = InputPattern::new(&input)?;
        pattern.literal = None;
        includes.push(pattern);
        scopes.push(package_dir.join_component(config));
    }

    let mut links = HashSet::new();
    let mut ancestors = HashMap::new();
    let mut default_links = HashSet::new();
    if let Some(indexed_links) = indexed_links {
        // Default indexed tasks reuse the repository's tracked modes and
        // untracked discovery instead of walking the package again.
        scopes.clear();
        for link in indexed_links {
            let key = AnchoredSystemPathBuf::relative_path_between(&package_dir, &link).to_unix();
            if hashes.contains_key(&key) {
                default_links.insert(link.clone());
            }
            links.insert(link);
        }
    } else {
        for key in hashes.keys() {
            let path = package_dir.join_unix_path(key);
            if let Some(link) = first_link(root, &path, &mut ancestors)? {
                // Git can supply tracked links even when ignored. An explicitly
                // selected key below an alias is not evidence that its siblings
                // are default inputs.
                let relative = root.anchor(&path)?.to_unix();
                if defaults
                    && (path == link
                        || !includes
                            .iter()
                            .any(|pattern| pattern.matches(relative.as_str())))
                {
                    default_links.insert(link.clone());
                }
                links.insert(link);
            }
        }
    }
    scopes.sort();
    let mut unique_scopes: Vec<AbsoluteSystemPathBuf> = Vec::new();
    for scope in scopes {
        if !unique_scopes.iter().any(|parent| scope.starts_with(parent)) {
            unique_scopes.push(scope);
        }
    }
    let reader = RepositoryReader::open(root)?;
    let mut expansion = Expansion {
        root,
        package_dir: &package_dir,
        reader: &reader,
        includes: &includes,
        excludes: &excludes,
        defaults,
        ignores: DefaultIgnores {
            root,
            reader: &reader,
            rules: HashMap::new(),
            ignored: HashMap::new(),
            default_links,
        },
        attrs_root: attrs.0,
        attrs: attrs.1,
        hashes,
    };
    for scope in unique_scopes {
        if let Some(link) = first_link(root, &scope, &mut ancestors)? {
            links.insert(link);
        } else {
            expansion.discover(&scope, &mut links)?;
        }
    }
    let mut links: Vec<_> = links.into_iter().collect();
    links.sort();
    for link in links {
        if !expansion.selected(&link, false)? {
            continue;
        }
        expansion
            .hashes
            .retain(|key, _| !package_dir.join_unix_path(key).starts_with(&link));
        expansion.visit(&link, &mut Vec::new())?;
    }
    Ok(())
}

fn first_link(
    root: &AbsoluteSystemPath,
    path: &AbsoluteSystemPath,
    cache: &mut HashMap<AbsoluteSystemPathBuf, Option<AbsoluteSystemPathBuf>>,
) -> Result<Option<AbsoluteSystemPathBuf>, Error> {
    if path == root {
        return Ok(None);
    }
    if let Some(link) = cache.get(path) {
        return Ok(link.clone());
    }
    root.anchor(path)?;
    let link = if let Some(parent) = path.parent()
        && let Some(link) = first_link(root, parent, cache)?
    {
        Some(link)
    } else {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Some(path.to_owned()),
            Ok(_) => None,
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(Error::hash_file(path.to_owned(), error)),
        }
    };
    cache.insert(path.to_owned(), link.clone());
    Ok(link)
}

struct DefaultIgnores<'a> {
    root: &'a AbsoluteSystemPath,
    reader: &'a RepositoryReader,
    rules: HashMap<AbsoluteSystemPathBuf, Gitignore>,
    ignored: HashMap<(AbsoluteSystemPathBuf, bool), bool>,
    default_links: HashSet<AbsoluteSystemPathBuf>,
}

impl DefaultIgnores<'_> {
    fn ignored(&mut self, path: &AbsoluteSystemPath, is_dir: bool) -> Result<bool, Error> {
        if path == self.root || self.default_links.contains(path) {
            return Ok(false);
        }
        let key = (path.to_owned(), is_dir);
        if let Some(ignored) = self.ignored.get(&key) {
            return Ok(*ignored);
        }
        let Some(parent) = path
            .parent()
            .filter(|parent| parent.as_std_path().starts_with(self.root.as_std_path()))
        else {
            return Ok(false);
        };
        let mut ignored = self.ignored(parent, true)?;
        if !ignored {
            let mut directory = Some(parent);
            while let Some(dir) =
                directory.filter(|dir| dir.as_std_path().starts_with(self.root.as_std_path()))
            {
                if !self.rules.contains_key(dir) {
                    let ignore_path = dir.join_component(".gitignore");
                    let mut builder = GitignoreBuilder::new(dir.as_std_path());
                    match self.reader.open_file(&ignore_path) {
                        Ok(Some(mut file)) => {
                            let mut contents = String::new();
                            file.read_to_string(&mut contents)?;
                            for line in contents.trim_start_matches('\u{feff}').lines() {
                                if let Err(error) = builder
                                    .add_line(Some(ignore_path.as_std_path().to_owned()), line)
                                {
                                    tracing::warn!(%ignore_path, %error, "invalid ignore pattern");
                                }
                            }
                        }
                        Ok(None) => {}
                        // An ignore file cannot grant access to external or
                        // metadata content. Ignore unavailable rules, not the input.
                        Err(error)
                            if matches!(
                                error.kind(),
                                ErrorKind::NotFound
                                    | ErrorKind::PermissionDenied
                                    | ErrorKind::InvalidInput
                            ) => {}
                        Err(error) => return Err(Error::hash_file(ignore_path, error)),
                    }
                    self.rules.insert(dir.to_owned(), builder.build()?);
                }
                let matched = self.rules[dir].matched(path.as_std_path(), is_dir);
                if !matched.is_none() {
                    ignored = matched.is_ignore();
                    break;
                }
                directory = dir.parent();
            }
        }
        self.ignored.insert(key, ignored);
        Ok(ignored)
    }
}

struct Expansion<'a> {
    root: &'a AbsoluteSystemPath,
    package_dir: &'a AbsoluteSystemPath,
    reader: &'a RepositoryReader,
    includes: &'a [InputPattern],
    excludes: &'a [InputPattern],
    defaults: bool,
    ignores: DefaultIgnores<'a>,
    attrs_root: &'a AbsoluteSystemPath,
    attrs: Option<&'a crlf::GitAttrs>,
    hashes: &'a mut GitHashes,
}

impl Expansion<'_> {
    fn excluded(&self, logical: &str) -> bool {
        self.excludes.iter().any(|pattern| pattern.matches(logical))
    }

    fn default_selected(
        &mut self,
        logical: &AbsoluteSystemPath,
        is_dir: bool,
    ) -> Result<bool, Error> {
        Ok(self.defaults
            && logical
                .as_std_path()
                .starts_with(self.package_dir.as_std_path())
            && !self.ignores.ignored(logical, is_dir)?)
    }

    fn selected(&mut self, logical: &AbsoluteSystemPath, is_dir: bool) -> Result<bool, Error> {
        let relative = self.root.anchor(logical)?.to_unix();
        if self.excluded(relative.as_str()) {
            return Ok(false);
        }
        Ok(self
            .includes
            .iter()
            .any(|pattern| pattern.reaches(relative.as_str()))
            || self.default_selected(logical, is_dir)?)
    }

    fn discover(
        &mut self,
        logical: &AbsoluteSystemPath,
        links: &mut HashSet<AbsoluteSystemPathBuf>,
    ) -> Result<(), Error> {
        // Ordinary authored metadata is already hashed by the caller. Do not
        // enumerate it here; symlink routes still go through `visit` instead.
        if self
            .root
            .anchor(logical)?
            .components()
            .any(|component| turbopath::is_git_metadata_component(component.as_str()))
            || !self.selected(logical, false)?
        {
            return Ok(());
        }
        let metadata = match std::fs::symlink_metadata(logical) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(Error::hash_file(logical.to_owned(), error)),
        };
        if metadata.file_type().is_symlink() {
            links.insert(logical.to_owned());
        } else if metadata.is_dir() && self.selected(logical, true)? {
            for name in self.reader.read_dir(logical)? {
                if !turbopath::is_git_metadata_component(&name) {
                    self.discover(&logical.join_component(&name), links)?;
                }
            }
        }
        Ok(())
    }

    fn visit(
        &mut self,
        logical: &AbsoluteSystemPath,
        ancestors: &mut Vec<AbsoluteSystemPathBuf>,
    ) -> Result<(), Error> {
        let relative = self.root.anchor(logical)?.to_unix();
        // Check logical reachability before resolving even the first link in
        // this branch. A selected alias does not select all of its siblings.
        if !self.selected(logical, false)? {
            return Ok(());
        }
        let physical = match self.reader.resolve(logical) {
            Ok(path) => path,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(Error::hash_file(logical.to_owned(), error)),
        };
        let metadata = std::fs::symlink_metadata(&physical)?;
        if metadata.is_dir() {
            if !self.selected(logical, true)? {
                return Ok(());
            }
            if ancestors.contains(&physical) {
                return Err(Error::hash_file(
                    logical.to_owned(),
                    std::io::Error::new(
                        ErrorKind::InvalidInput,
                        "symlink directory cycle in task inputs",
                    ),
                ));
            }
            ancestors.push(physical);
            for name in self.reader.read_dir(logical)? {
                if turbopath::is_git_metadata_component(&name) {
                    continue;
                }
                self.visit(&logical.join_component(&name), ancestors)?;
            }
            ancestors.pop();
        } else if metadata.is_file()
            && (self.default_selected(logical, false)?
                || self
                    .includes
                    .iter()
                    .any(|pattern| pattern.matches(relative.as_str())))
            && let Some(mut file) = self.reader.open_file(logical)?
        {
            let attr_path = self.attrs_root.anchor(logical)?.to_unix();
            let attr = self
                .attrs
                .map(|attrs| attrs.resolve_text_attr(attr_path.as_str()))
                .unwrap_or(crlf::TextAttr::Unspecified);
            let len = file.metadata()?.len();
            let hash = crlf::hash_file_for_cache(logical, &mut file, len, attr)?;
            let key =
                AnchoredSystemPathBuf::relative_path_between(self.package_dir, logical).to_unix();
            self.hashes.insert(key, hash);
        }
        Ok(())
    }
}
