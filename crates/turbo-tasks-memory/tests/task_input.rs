use anyhow::Result;
use turbo_tasks::{CompletionVc, TaskInput};
use turbo_tasks_testing::{register, run};

register!();

#[derive(Clone, TaskInput, Eq, PartialEq, Debug)]
struct NoFields;

#[turbo_tasks::function]
async fn no_fields(input: NoFields) -> CompletionVc {
    // Can't fail.
    assert_eq!(input, NoFields);
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput)]
struct OneUnnamedField(u32);

#[turbo_tasks::function]
async fn one_unnamed_field(input: OneUnnamedField) -> CompletionVc {
    assert_eq!(input.0, 42);
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput)]
struct MultipleUnnamedFields(u32, String);

#[turbo_tasks::function]
async fn multiple_unnamed_fields(input: MultipleUnnamedFields) -> CompletionVc {
    assert_eq!(input.0, 42);
    assert_eq!(input.1, "42");
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput)]
struct OneNamedField {
    named: u32,
}

#[turbo_tasks::function]
async fn one_named_field(input: OneNamedField) -> CompletionVc {
    assert_eq!(input.named, 42);
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput)]
struct MultipleNamedFields {
    named: u32,
    other: String,
}

#[turbo_tasks::function]
async fn multiple_named_fields(input: MultipleNamedFields) -> CompletionVc {
    assert_eq!(input.named, 42);
    assert_eq!(input.other, "42");
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput)]
struct GenericField<T>(T);

#[turbo_tasks::function]
async fn generic_field1(input: GenericField<u32>) -> CompletionVc {
    assert_eq!(input.0, 42);
    CompletionVc::immutable()
}

#[turbo_tasks::function]
async fn generic_field2(input: GenericField<String>) -> CompletionVc {
    assert_eq!(input.0, "42");
    CompletionVc::immutable()
}

// This can't actually be tested at runtime because such an enum can't be
// constructed. However, the macro expansion is tested.
#[derive(Clone, TaskInput)]
enum NoVariants {}

#[turbo_tasks::function]
async fn no_variants(_input: NoVariants) -> CompletionVc {
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput, Eq, PartialEq, Debug)]
enum OneVariant {
    Variant,
}

#[turbo_tasks::function]
async fn one_variant(input: OneVariant) -> CompletionVc {
    // Can't fail.
    assert_eq!(input, OneVariant::Variant);
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput, PartialEq, Eq, Debug)]
enum MultipleVariants {
    Variant1,
    Variant2,
}

#[turbo_tasks::function]
async fn multiple_variants(input: MultipleVariants) -> CompletionVc {
    assert_eq!(input, MultipleVariants::Variant2);
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput, Eq, PartialEq, Debug)]
enum MultipleVariantsAndHeterogeneousFields {
    Variant1,
    Variant2(u32),
    Variant3 { named: u32 },
    Variant4(u32, String),
    Variant5 { named: u32, other: String },
}

#[turbo_tasks::function]
async fn multiple_variants_and_heterogeneous_fields(
    input: MultipleVariantsAndHeterogeneousFields,
) -> CompletionVc {
    assert_eq!(
        input,
        MultipleVariantsAndHeterogeneousFields::Variant5 {
            named: 42,
            other: "42".into(),
        }
    );
    CompletionVc::immutable()
}

#[derive(Clone, TaskInput, Eq, PartialEq, Debug)]
enum NestedVariants {
    Variant1,
    Variant2(MultipleVariantsAndHeterogeneousFields),
    Variant3 { named: OneVariant },
    Variant4(OneVariant, String),
    Variant5 { named: OneVariant, other: String },
}

#[turbo_tasks::function]
async fn nested_variants1(input: NestedVariants) -> CompletionVc {
    assert_eq!(
        input,
        NestedVariants::Variant5 {
            named: OneVariant::Variant,
            other: "42".into(),
        }
    );
    CompletionVc::immutable()
}

#[turbo_tasks::function]
async fn nested_variants2(input: NestedVariants) -> CompletionVc {
    assert_eq!(
        input,
        NestedVariants::Variant2(MultipleVariantsAndHeterogeneousFields::Variant5 {
            named: 42,
            other: "42".into(),
        })
    );
    CompletionVc::immutable()
}

#[tokio::test]
async fn tests() {
    run! {
        no_fields(NoFields).await?;
        one_unnamed_field(OneUnnamedField(42)).await?;
        multiple_unnamed_fields(MultipleUnnamedFields(42, "42".into())).await?;
        one_named_field(OneNamedField { named: 42 }).await?;
        multiple_named_fields(MultipleNamedFields { named: 42, other: "42".into() }).await?;
        generic_field1(GenericField(42)).await?;
        generic_field2(GenericField("42".into())).await?;

        one_variant(OneVariant::Variant).await?;
        multiple_variants(MultipleVariants::Variant2).await?;
        multiple_variants_and_heterogeneous_fields(MultipleVariantsAndHeterogeneousFields::Variant5 { named: 42, other: "42".into() }).await?;
        nested_variants1(NestedVariants::Variant5 { named: OneVariant::Variant, other: "42".into() }).await?;
        nested_variants2(NestedVariants::Variant2(MultipleVariantsAndHeterogeneousFields::Variant5 { named: 42, other: "42".into() })).await?;
    }
}
