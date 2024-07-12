use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{parse_macro_input, spanned::Spanned, Data, DataEnum, DataStruct, DeriveInput};

pub fn derive_task_input(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    let ident = &derive_input.ident;
    let generics = &derive_input.generics;

    if let Some(where_clause) = &generics.where_clause {
        // NOTE(alexkirsz) We could support where clauses and generic parameters bounds
        // in the future, but for simplicity's sake, we don't support them yet.
        where_clause
            .span()
            .unwrap()
            .error("the TaskInput derive macro does not support where clauses yet")
            .emit();
    }

    for param in &generics.params {
        match param {
            syn::GenericParam::Type(param) => {
                if !param.bounds.is_empty() {
                    // NOTE(alexkirsz) See where clause above.
                    param
                        .span()
                        .unwrap()
                        .error(
                            "the TaskInput derive macro does not support generic parameters \
                             bounds yet",
                        )
                        .emit();
                }
            }
            syn::GenericParam::Lifetime(param) => {
                param
                    .span()
                    .unwrap()
                    .error("the TaskInput derive macro does not support generic lifetimes")
                    .emit();
            }
            syn::GenericParam::Const(param) => {
                // NOTE(alexkirsz) Ditto: not supported yet for simplicity's sake.
                param
                    .span()
                    .unwrap()
                    .error("the TaskInput derive macro does not support const generics yet")
                    .emit();
            }
        }
    }

    let is_resolved_impl;
    let is_transient_impl;
    let resolve_impl;

    match &derive_input.data {
        Data::Enum(DataEnum { variants, .. }) => {
            let variants = if variants.is_empty() {
                vec![(
                    quote! { _ => true },
                    quote! { _ => false },
                    quote! { _ => unreachable!() },
                )]
            } else {
                variants
                    .iter()
                    .map(|variant| {
                        let variant_ident = &variant.ident;
                        let pattern;
                        let construct;
                        let field_names: Vec<_>;
                        match &variant.fields {
                            syn::Fields::Named(fields) => {
                                field_names = fields
                                    .named
                                    .iter()
                                    .map(|field| field.ident.clone().unwrap())
                                    .collect();
                                pattern = quote! {
                                    #ident::#variant_ident { #(#field_names,)* }
                                };
                                construct = quote! {
                                    #ident::#variant_ident { #(#field_names,)* }
                                };
                            }
                            syn::Fields::Unnamed(fields) => {
                                field_names = (0..fields.unnamed.len())
                                    .map(|i| Ident::new(&format!("field{}", i), fields.span()))
                                    .collect();
                                pattern = quote! {
                                    #ident::#variant_ident ( #(#field_names,)* )
                                };
                                construct = quote! {
                                    #ident::#variant_ident ( #(#field_names,)* )
                                };
                            }
                            syn::Fields::Unit => {
                                field_names = vec![];
                                pattern = quote! {
                                    #ident::#variant_ident
                                };
                                construct = quote! {
                                    #ident::#variant_ident
                                };
                            }
                        }
                        (
                            quote! {
                                #pattern => {
                                    #(#field_names.is_resolved() &&)* true
                                },
                            },
                            quote! {
                                #pattern => {
                                    #(#field_names.is_transient() ||)* false
                                },
                            },
                            quote! {
                                #pattern => {
                                    #(
                                        let #field_names = #field_names.resolve().await?;
                                    )*
                                    #construct
                                },
                            },
                        )
                    })
                    .collect::<Vec<_>>()
            };

            let is_resolve_variants = variants.iter().map(|(is_resolved, _, _)| is_resolved);
            let is_transient_variants = variants.iter().map(|(_, is_transient, _)| is_transient);
            let resolve_variants = variants.iter().map(|(_, _, resolve)| resolve);

            is_resolved_impl = quote! {
                match self {
                    #(
                        #is_resolve_variants
                    )*
                }
            };
            is_transient_impl = quote! {
                match self {
                    #(
                        #is_transient_variants
                    )*
                }
            };
            resolve_impl = quote! {
                Ok(match self {
                    #(
                        #resolve_variants
                    )*
                })
            };
        }
        Data::Struct(DataStruct { fields, .. }) => {
            let destruct;
            let construct;
            let field_names: Vec<Ident>;
            match fields {
                syn::Fields::Named(fields) => {
                    field_names = fields
                        .named
                        .iter()
                        .map(|field| field.ident.clone().unwrap())
                        .collect();
                    destruct = quote! {
                        let #ident { #(#field_names,)* } = self;
                    };
                    construct = quote! {
                        #ident { #(#field_names,)* }
                    };
                }
                syn::Fields::Unnamed(fields) => {
                    field_names = (0..fields.unnamed.len())
                        .map(|i| Ident::new(&format!("field{}", i), fields.span()))
                        .collect();
                    destruct = quote! {
                        let #ident ( #(#field_names,)* ) = self;
                    };
                    construct = quote! {
                        #ident ( #(#field_names,)* )
                    };
                }
                syn::Fields::Unit => {
                    field_names = vec![];
                    destruct = quote! {};
                    construct = quote! {
                        #ident
                    };
                }
            }

            is_resolved_impl = quote! {
                #destruct
                #(#field_names.is_resolved() &&)* true
            };
            is_transient_impl = quote! {
                #destruct
                #(#field_names.is_transient() ||)* false
            };
            resolve_impl = quote! {
                #destruct
                #(
                    let #field_names = #field_names.resolve().await?;
                )*
                Ok(#construct)
            };
        }
        _ => {
            derive_input
                .span()
                .unwrap()
                .error("unsupported syntax")
                .emit();

            is_resolved_impl = quote! {};
            is_transient_impl = quote! {};
            resolve_impl = quote! {};
        }
    };

    let generic_params: Vec<_> = generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(param) => Some(param),
            _ => {
                // We already report an error for this above.
                None
            }
        })
        .collect();

    quote! {
        #[turbo_tasks::macro_helpers::async_trait]
        impl #generics turbo_tasks::TaskInput for #ident #generics
        where
            #(#generic_params: turbo_tasks::TaskInput,)*
        {
            #[allow(non_snake_case)]
            #[allow(unreachable_code)] // This can occur for enums with no variants.
            fn is_resolved(&self) -> bool {
                #is_resolved_impl
            }

            #[allow(non_snake_case)]
            #[allow(unreachable_code)] // This can occur for enums with no variants.
            fn is_transient(&self) -> bool {
                #is_transient_impl
            }

            #[allow(non_snake_case)]
            #[allow(unreachable_code)] // This can occur for enums with no variants.
            async fn resolve(&self) -> turbo_tasks::Result<Self> {
                #resolve_impl
            }
        }
    }
    .into()
}
