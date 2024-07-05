use turbo_tasks::{ResolvedValue, ResolvedVc, Vc};

#[derive(ResolvedValue)]
struct ContainsResolvedVc {
    a: ResolvedVc<i32>,
}

#[derive(ResolvedValue)]
struct ContainsVc {
    a: Vc<i32>,
}

#[derive(ResolvedValue)]
struct ContainsMixedVc {
    a: Vc<i32>,
    b: ResolvedVc<i32>,
}
