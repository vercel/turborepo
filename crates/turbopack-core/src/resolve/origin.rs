use anyhow::Result;
use turbo_tasks::{Value, Vc};
use turbo_tasks_fs::FileSystemPath;

use super::{options::ResolveOptions, parse::Request, ResolveResult};
use crate::{asset::AssetOption, context::AssetContext, reference_type::ReferenceType};

/// A location where resolving can occur from. It carries some meta information
/// that are needed for resolving from here.
#[turbo_tasks::value_trait]
pub trait ResolveOrigin {
    /// The origin path where resolving starts. This is pointing to a file,
    /// since that might be needed to infer custom resolving options for that
    /// specific file. But usually only the directory is relevant for the real
    /// resolving.
    fn origin_path(self: Vc<Self>) -> Vc<FileSystemPath>;

    /// The AssetContext that carries the configuration for building that
    /// subgraph.
    fn context(self: Vc<Self>) -> Vc<Box<dyn AssetContext>>;

    /// Get an inner asset form this origin that doesn't require resolving but
    /// is directly attached
    fn get_inner_asset(self: Vc<Self>, _request: Vc<Request>) -> Vc<AssetOption> {
        Vc::cell(None)
    }
}

#[turbo_tasks::value_impl]
impl ResolveOrigin {
    // TODO it would be nice if these methods can be moved to the trait to allow
    // overriding it, but currently this is not possible due to the way transitions
    // work. Maybe transitions should be decorators on ResolveOrigin?

    /// Resolve to an asset from that origin. Custom resolve options can be
    /// passed. Otherwise provide `origin.resolve_options()` unmodified.
    #[turbo_tasks::function]
    pub async fn resolve_asset(
        self: Vc<Self>,
        request: Vc<Request>,
        options: Vc<ResolveOptions>,
        reference_type: Value<ReferenceType>,
    ) -> Result<Vc<ResolveResult>> {
        if let Some(asset) = *self.get_inner_asset(request).await? {
            return Ok(ResolveResult::asset(asset).cell());
        }
        Ok(self
            .context()
            .resolve_asset(self.origin_path(), request, options, reference_type))
    }

    /// Get the resolve options that apply for this origin.
    #[turbo_tasks::function]
    pub fn resolve_options(
        self: Vc<Self>,
        reference_type: Value<ReferenceType>,
    ) -> Vc<ResolveOptions> {
        self.context()
            .resolve_options(self.origin_path(), reference_type)
    }

    /// Adds a transition that is used for resolved assets.
    #[turbo_tasks::function]
    pub fn with_transition(self: Vc<Self>, transition: String) -> Vc<Self> {
        ResolveOriginWithTransition {
            previous: self,
            transition: transition.to_string(),
        }
        .cell()
        .into()
    }
}

/// A resolve origin for some path and context without additional modifications.
#[turbo_tasks::value]
pub struct PlainResolveOrigin {
    context: Vc<Box<dyn AssetContext>>,
    origin_path: Vc<FileSystemPath>,
}

#[turbo_tasks::value_impl]
impl PlainResolveOrigin {
    #[turbo_tasks::function]
    pub fn new(context: Vc<Box<dyn AssetContext>>, origin_path: Vc<FileSystemPath>) -> Vc<Self> {
        PlainResolveOrigin {
            context,
            origin_path,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ResolveOrigin for PlainResolveOrigin {
    #[turbo_tasks::function]
    fn origin_path(&self) -> Vc<FileSystemPath> {
        self.origin_path
    }

    #[turbo_tasks::function]
    fn context(&self) -> Vc<Box<dyn AssetContext>> {
        self.context
    }
}

/// Wraps a ResolveOrigin to add a transition.
#[turbo_tasks::value]
struct ResolveOriginWithTransition {
    previous: Vc<Box<dyn ResolveOrigin>>,
    transition: String,
}

#[turbo_tasks::value_impl]
impl ResolveOrigin for ResolveOriginWithTransition {
    #[turbo_tasks::function]
    fn origin_path(&self) -> Vc<FileSystemPath> {
        self.previous.origin_path()
    }

    #[turbo_tasks::function]
    fn context(&self) -> Vc<Box<dyn AssetContext>> {
        self.previous.context().with_transition(&self.transition)
    }

    #[turbo_tasks::function]
    fn get_inner_asset(&self, request: Vc<Request>) -> Vc<AssetOption> {
        self.previous.get_inner_asset(request)
    }
}
