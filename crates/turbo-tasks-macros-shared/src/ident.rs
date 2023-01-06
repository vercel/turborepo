use quote::ToTokens;
use syn::{spanned::Spanned, Ident, Path, Type, TypeReference};

pub fn get_register_value_type_ident(struct_ident: &Ident) -> Ident {
    Ident::new(
        &format!("__register_{struct_ident}_value_type"),
        struct_ident.span(),
    )
}

pub fn get_register_trait_methods_ident(trait_ident: &Ident, struct_ident: &Ident) -> Ident {
    Ident::new(
        &format!("__register_{struct_ident}_{trait_ident}_trait_methods"),
        trait_ident.span(),
    )
}

pub fn get_native_function_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_FUNCTION", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_native_function_id_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_FUNCTION_ID", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_trait_type_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_TRAIT_TYPE", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_impl_function_ident(struct_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_{}_FUNCTION",
            struct_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_inherent_impl_function_ident(ty_ident: &Ident, fn_ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_{}_FUNCTION",
            ty_ident.to_string().to_uppercase(),
            fn_ident.to_string().to_uppercase()
        ),
        fn_ident.span(),
    )
}

pub fn get_inherent_impl_function_id_ident(ty_ident: &Ident, fn_ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_{}_FUNCTION_ID",
            ty_ident.to_string().to_uppercase(),
            fn_ident.to_string().to_uppercase()
        ),
        fn_ident.span(),
    )
}

pub fn get_trait_impl_function_ident(
    struct_ident: &Ident,
    trait_ident: &Ident,
    ident: &Ident,
) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_TRAIT_{}_{}_FUNCTION",
            struct_ident.to_string().to_uppercase(),
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_trait_impl_function_id_ident(
    struct_ident: &Ident,
    trait_ident: &Ident,
    ident: &Ident,
) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_TRAIT_{}_{}_FUNCTION_ID",
            struct_ident.to_string().to_uppercase(),
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_static_trait_impl_function_ident(
    struct_ident: &Ident,
    trait_ident: &Ident,
    ident: &Ident,
) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_TRAIT_{}_{}_FUNCTION_STATIC",
            struct_ident.to_string().to_uppercase(),
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_static_trait_impl_function_id_ident(
    struct_ident: &Ident,
    trait_ident: &Ident,
    ident: &Ident,
) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_TRAIT_{}_{}_FUNCTION_ID_STATIC",
            struct_ident.to_string().to_uppercase(),
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_internal_trait_impl_function_ident(trait_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &format!("__trait_call_{trait_ident}_{ident}"),
        trait_ident.span(),
    )
}

pub fn get_last_path_ident(path: &Path) -> &Ident {
    &path.segments.last().unwrap().ident
}

pub fn get_type_ident(ty: &Type) -> Option<&Ident> {
    match ty {
        // T
        Type::Path(path) => Some(get_last_path_ident(&path.path)),
        // dyn T and &dyn T
        Type::Reference(TypeReference {
            lifetime: None,
            mutability: None,
            elem: box Type::TraitObject(trait_object),
            ..
        })
        | Type::TraitObject(trait_object) => {
            if trait_object.bounds.len() > 1 {
                trait_object
                    .span()
                    .unwrap()
                    .error(
                        "#[turbo_tasks::value_impl] does not support trait objects with more than \
                         one bound",
                    )
                    .emit();
                return None;
            }

            let trait_path = match &trait_object.bounds[0] {
                syn::TypeParamBound::Trait(trait_bound) => &trait_bound.path,
                _ => {
                    // The compiler should have already caught this.
                    return None;
                }
            };

            Some(get_last_path_ident(trait_path))
        }
        _ => {
            ty.span()
                .unwrap()
                .error(format!(
                    "#[turbo_tasks::value_impl] does not support the type {}, expected T or &dyn \
                     Trait",
                    ty.to_token_stream()
                ))
                .emit();
            None
        }
    }
}

pub fn get_trait_impl_function_ident2(ty: &Type, trait_path: &Path, ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_IMPL_TRAIT_{}_{}_FUNCTION",
            ty.to_token_stream().to_string().to_uppercase(),
            trait_path.to_token_stream().to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_trait_default_impl_function_ident(trait_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_DEFAULT_IMPL_{}_FUNCTION",
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_trait_type_id_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_TRAIT_TYPE_ID", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_trait_default_impl_function_id_ident(trait_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &format!(
            "{}_DEFAULT_IMPL_{}_FUNCTION_ID",
            trait_ident.to_string().to_uppercase(),
            ident.to_string().to_uppercase()
        ),
        ident.span(),
    )
}

pub fn get_value_type_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_VALUE_TYPE", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_value_type_id_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_VALUE_TYPE_ID", ident.to_string().to_uppercase()),
        ident.span(),
    )
}

pub fn get_value_type_init_ident(ident: &Ident) -> Ident {
    Ident::new(
        &format!("{}_VALUE_TYPE_INIT", ident.to_string().to_uppercase()),
        ident.span(),
    )
}
