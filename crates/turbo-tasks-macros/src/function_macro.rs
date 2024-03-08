use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{parse_macro_input, parse_quote, ExprPath, ItemFn};
use turbo_tasks_macros_shared::{get_native_function_id_ident, get_native_function_ident};

use crate::func::{DefinitionContext, NativeFn, TurboFn};

/// the same module exists in turbo-tasks-build but due to its size we
/// just copy the relevant parts in both places
mod ignored {
    use std::{cell::OnceCell, collections::HashSet};
    // a newline-separated list of task names to opt-out of turbo tracking
    const IGNORE_TASKS_RAW: Option<&str> = std::option_env!("TURBO_IGNORE_TASKS");
    const IGNORE_TASKS: OnceCell<HashSet<&'static str>> = OnceCell::new();

    pub fn task_ignored(name: &str) -> bool {
        IGNORE_TASKS
            .get_or_init(|| IGNORE_TASKS_RAW.unwrap_or_default().split(',').collect())
            .contains(name)
    }
}

pub fn function(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemFn);

    // TODO: ideally here we would be able to match on the entire module path
    //       however macros are evaluated in a different context and we don't
    //       have access to it. in the mean time, just use the function name.
    //       the likelihood that two functions with the same name exist is low
    //       and, if we do need to disable just one of them, can be mitigated
    //       through naming
    let name = item.sig.ident.to_string();
    if ignored::task_ignored(&name) {
        return quote! { #item }.into();
    }

    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = &item;

    let Some(turbo_fn) = TurboFn::new(sig, DefinitionContext::NakedFn) else {
        return quote! {
            // An error occurred while parsing the function signature.
        }
        .into();
    };

    let ident = &sig.ident;

    let inline_function_ident = Ident::new(&format!("{ident}_inline_function"), ident.span());
    let inline_function_path: ExprPath = parse_quote! { #inline_function_ident };
    let mut inline_signature = sig.clone();
    inline_signature.ident = inline_function_ident;

    let native_fn = NativeFn::new(&ident.to_string(), &inline_function_path);
    let native_function_ident = get_native_function_ident(ident);
    let native_function_ty = native_fn.ty();
    let native_function_def = native_fn.definition();

    let native_function_id_ident = get_native_function_id_ident(ident);
    let native_function_id_ty = native_fn.id_ty();
    let native_function_id_def = native_fn.id_definition(&native_function_ident.clone().into());

    let exposed_signature = turbo_fn.signature();
    let exposed_block = turbo_fn.static_block(&native_function_id_ident);

    quote! {
        #(#attrs)*
        #vis #exposed_signature #exposed_block

        #(#attrs)*
        #[doc(hidden)]
        #inline_signature #block

        #[doc(hidden)]
        pub(crate) static #native_function_ident: #native_function_ty = #native_function_def;

        #[doc(hidden)]
        pub(crate) static #native_function_id_ident: #native_function_id_ty = #native_function_id_def;
    }
    .into()
}
