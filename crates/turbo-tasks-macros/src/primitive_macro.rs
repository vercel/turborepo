use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, LitStr, Result, Token, Type,
};
use turbo_tasks_macros_shared::{
    get_register_value_type_ident, get_type_ident, get_value_type_id_ident, get_value_type_ident,
    get_value_type_init_ident,
};

#[derive(Debug)]
struct PrimitiveInput {
    ty: Type,
    ident: Option<LitStr>,
}

impl Parse for PrimitiveInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let ty: Type = input.parse()?;
        let colon: Option<Token![,]> = input.parse()?;

        let ident = if colon.is_some() {
            Some(input.parse()?)
        } else {
            None
        };

        Ok(PrimitiveInput { ty, ident })
    }
}

// TODO(alexkirsz) Most of this should be shared with `value_macro`.
pub fn primitive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as PrimitiveInput);

    let ty = input.ty;
    let Some(ident) = input
        .ident
        .as_ref()
        .map(|ident| Ident::new(&ident.value(), ident.span()))
        .or_else(|| get_type_ident(&ty).cloned()) else {
        return quote! {
            // An error occurred while parsing the ident.
        }.into();
    };

    let value_type_init_ident = get_value_type_init_ident(&ident);
    let value_type_ident = get_value_type_ident(&ident);
    let value_type_id_ident = get_value_type_id_ident(&ident);
    let register_value_type_ident = get_register_value_type_ident(&ident);

    let new_value_type = quote! {
        turbo_tasks::ValueType::new_with_any_serialization::<#ty>()
    };

    quote! {
        #[doc(hidden)]
        static #value_type_init_ident: turbo_tasks::macro_helpers::OnceCell<
            turbo_tasks::ValueType,
        > = turbo_tasks::macro_helpers::OnceCell::new();
        #[doc(hidden)]
        pub(crate) static #value_type_ident: turbo_tasks::macro_helpers::Lazy<&turbo_tasks::ValueType> =
            turbo_tasks::macro_helpers::Lazy::new(|| {
                #value_type_init_ident.get_or_init(|| {
                    panic!(
                        concat!(
                            stringify!(#value_type_ident),
                            " has not been initialized (this should happen via the generated register function)"
                        )
                    )
                })
            });
        #[doc(hidden)]
        static #value_type_id_ident: turbo_tasks::macro_helpers::Lazy<turbo_tasks::ValueTypeId> =
            turbo_tasks::macro_helpers::Lazy::new(|| {
                turbo_tasks::registry::get_value_type_id(*#value_type_ident)
            });


        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub(crate) fn #register_value_type_ident(
            global_name: &'static str,
            f: impl FnOnce(&mut turbo_tasks::ValueType),
        ) {
            #value_type_init_ident.get_or_init(|| {
                let mut value = #new_value_type;
                f(&mut value);
                value
            }).register(global_name);
        }

        unsafe impl turbo_tasks::VcValueType for #ty {
            type Read = turbo_tasks::VcTransparentRead<#ty, #ty>;
            type CellMode = turbo_tasks::VcCellSharedMode<#ty>;

            fn get_value_type_id() -> turbo_tasks::ValueTypeId {
                *#value_type_id_ident
            }
        }
    }.into()
}
