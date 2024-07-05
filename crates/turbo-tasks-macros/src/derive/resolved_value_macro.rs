use either::Either;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::{parse_macro_input, spanned::Spanned, Data, DeriveInput, Generics, Type};

pub fn derive_resolved_value(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    let ident = &derive_input.ident;

    let assertions: Vec<_> = iter_data_fields(&derive_input.data)
        .map(|f| assert_field_resolved_value(&derive_input.generics, &f.ty))
        .collect();

    let (impl_generics, ty_generics, where_clause) = derive_input.generics.split_for_impl();
    quote! {
        unsafe impl #impl_generics ::turbo_tasks::ResolvedValue
            for #ident #ty_generics #where_clause {}
        #(#assertions)*
    }
    .into()
}

fn iter_data_fields(data: &Data) -> impl Iterator<Item = &syn::Field> {
    match data {
        Data::Struct(ds) => Either::Left(ds.fields.iter()),
        Data::Enum(de) => Either::Right(Either::Left(de.variants.iter().flat_map(|v| &v.fields))),
        Data::Union(du) => Either::Right(Either::Right(du.fields.named.iter())),
    }
}

fn assert_field_resolved_value(generics: &Generics, ty: &Type) -> TokenStream2 {
    // this technique is based on the trick used by
    // `static_assertions::assert_impl_all`, but extended to support generics.
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote_spanned! {
        ty.span() =>
        const _: fn() = || {
            // create this struct just to hold onto our generics
            struct StaticAssertion #impl_generics #where_clause;
            impl #impl_generics StaticAssertion #ty_generics #where_clause {
                fn assert_impl_resolved_value<ExpectedResolvedValue: ResolvedValue + ?Sized>() {}
                fn call_site() {
                    // this call is only valid if ty is a ResolvedValue
                    Self::assert_impl_resolved_value::<#ty>();
                }
            }
        };
    }
}
